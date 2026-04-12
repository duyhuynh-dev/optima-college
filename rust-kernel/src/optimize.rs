//! Local schedule optimization: candidate generation, scoring (aligned with Go orchestrator), and conflict filtering.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde::Deserialize;
use serde_json::Value;

use crate::conflicts::{detect_conflicts, load_meetings_from_csv, parse_time_to_minutes, MeetingRecord};
use crate::weekly_bitmap::{MeetingBlock, WeeklyOccupancy};

#[derive(Debug, Clone)]
pub struct ScoreWeights {
    pub weekly: f64,
    pub evening: f64,
    pub early: f64,
    pub back_to_back: f64,
    pub busy_day: f64,
}

#[derive(Debug, Clone)]
pub struct ScheduleOption {
    pub id: String,
    pub sections: Vec<String>,
    pub expected_utility: f64,
    pub stress_score: f64,
    pub academic_load_score: f64,
    pub lifestyle_penalty_score: f64,
}

#[derive(Debug, Clone)]
pub struct OptimizeParams {
    pub k: i32,
    pub max_results: i32,
    pub max_per_subject: i32,
    pub earliest_start_minutes: i32,
    pub subject_whitelist: Vec<String>,
    pub subject_blacklist: Vec<String>,
    pub weights: ScoreWeights,
    pub pareto: bool,
    pub pareto_mode: String,
    pub pareto_epsilon: f64,
    pub max_candidates: i32,
    /// Hard lower bound on sum of selected section credits; `0` = disabled.
    pub min_total_credits: f64,
    /// Hard upper bound on sum of selected section credits; `0` = disabled.
    pub max_total_credits: f64,
}

fn default_section_credits() -> f64 {
    1.0
}

#[derive(Debug, Clone, Deserialize)]
struct SectionRow {
    subject_code: String,
    course_code: String,
    section: String,
    #[serde(default = "default_section_credits")]
    credits: f64,
}

fn default_weights() -> ScoreWeights {
    ScoreWeights {
        weekly: 0.35,
        evening: 0.20,
        early: 0.15,
        back_to_back: 0.15,
        busy_day: 0.15,
    }
}

pub fn normalize_weights(w: ScoreWeights) -> ScoreWeights {
    let sum = w.weekly + w.evening + w.early + w.back_to_back + w.busy_day;
    if sum <= 1e-9 {
        return default_weights();
    }
    ScoreWeights {
        weekly: w.weekly / sum,
        evening: w.evening / sum,
        early: w.early / sum,
        back_to_back: w.back_to_back / sum,
        busy_day: w.busy_day / sum,
    }
}

fn round_float(v: f64, places: i32) -> f64 {
    let p = 10_f64.powi(places);
    (v * p).round() / p
}

#[derive(Debug, Deserialize)]
struct CourseCsvRow {
    course_code: String,
    #[serde(default)]
    prereq_groups: String,
}

fn normalize_prereq_clauses(raw: &str) -> Result<Vec<Vec<String>>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "[]" {
        return Ok(Vec::new());
    }
    let v: Value = serde_json::from_str(trimmed).map_err(|e| format!("prereq_groups json: {e}"))?;
    let arr = v.as_array().ok_or_else(|| "prereq_groups must be a JSON array".to_string())?;
    let mut out: Vec<Vec<String>> = Vec::with_capacity(arr.len());
    for item in arr {
        let inner = item.as_array().ok_or_else(|| "prereq_groups clause must be array".to_string())?;
        let mut g: Vec<String> = Vec::new();
        for x in inner {
            let s = x
                .as_str()
                .ok_or_else(|| "prereq_groups code must be string".to_string())?
                .trim()
                .to_uppercase();
            if !s.is_empty() && !g.contains(&s) {
                g.push(s);
            }
        }
        if !g.is_empty() {
            out.push(g);
        }
    }
    Ok(out)
}

fn load_prereq_clauses_map(path: &Path) -> Result<HashMap<String, Vec<Vec<String>>>, String> {
    let mut reader = csv::Reader::from_path(path)
        .map_err(|e| format!("failed to open courses csv {}: {e}", path.display()))?;
    let mut out: HashMap<String, Vec<Vec<String>>> = HashMap::new();
    for row in reader.deserialize::<CourseCsvRow>() {
        let row = row.map_err(|e| format!("courses csv row: {e}"))?;
        let code = row.course_code.trim().to_uppercase();
        if code.is_empty() {
            continue;
        }
        let clauses = normalize_prereq_clauses(&row.prereq_groups)?;
        out.insert(code, clauses);
    }
    Ok(out)
}

fn companion_courses_csv(sections_path: &Path) -> Option<PathBuf> {
    let name = sections_path.file_name()?.to_str()?;
    let rest = name.strip_prefix("sections_")?;
    let term = rest.strip_suffix(".csv")?;
    let p = sections_path.parent()?.join(format!("courses_{term}.csv"));
    if p.is_file() {
        Some(p)
    } else {
        None
    }
}

/// Each AND-clause (OR-group) must be satisfied by some alternative `p` that appears in the
/// schedule and whose own prerequisite groups are recursively satisfied (transitive closure).
fn schedule_satisfies_prereq_groups(
    selected_courses: &HashSet<String>,
    by_course: &HashMap<String, Vec<Vec<String>>>,
) -> bool {
    let mut memo: HashMap<String, bool> = HashMap::new();
    let mut visiting: HashSet<String> = HashSet::new();
    for course in selected_courses.iter() {
        if !course_prereqs_transitive_satisfied(
            course,
            selected_courses,
            by_course,
            &mut memo,
            &mut visiting,
        ) {
            return false;
        }
    }
    true
}

fn course_prereqs_transitive_satisfied(
    course: &str,
    selected: &HashSet<String>,
    by_course: &HashMap<String, Vec<Vec<String>>>,
    memo: &mut HashMap<String, bool>,
    visiting: &mut HashSet<String>,
) -> bool {
    if let Some(&v) = memo.get(course) {
        return v;
    }
    if visiting.contains(course) {
        return false;
    }
    let Some(clauses) = by_course.get(course) else {
        memo.insert(course.to_string(), true);
        return true;
    };
    if clauses.is_empty() {
        memo.insert(course.to_string(), true);
        return true;
    }

    visiting.insert(course.to_string());
    for or_group in clauses {
        if or_group.is_empty() {
            continue;
        }
        let mut group_ok = false;
        for alt in or_group {
            if selected.contains(alt)
                && course_prereqs_transitive_satisfied(alt, selected, by_course, memo, visiting)
            {
                group_ok = true;
                break;
            }
        }
        if !group_ok {
            visiting.remove(course);
            memo.insert(course.to_string(), false);
            return false;
        }
    }
    visiting.remove(course);
    memo.insert(course.to_string(), true);
    true
}

fn load_sections_csv(path: &Path, max_rows: usize) -> Result<Vec<SectionRow>, String> {
    let mut reader = csv::Reader::from_path(path)
        .map_err(|e| format!("failed to open sections csv {}: {e}", path.display()))?;
    let mut seen = HashSet::new();
    let mut rows = Vec::new();
    for row in reader.deserialize::<SectionRow>() {
        let row = row.map_err(|e| format!("sections csv row: {e}"))?;
        let key = format!("{}-{}", row.course_code, row.section);
        if seen.contains(&key) {
            continue;
        }
        seen.insert(key);
        rows.push(row);
        if max_rows > 0 && rows.len() >= max_rows {
            break;
        }
    }
    Ok(rows)
}

fn build_meeting_blocks_and_starts(meetings: &[MeetingRecord]) -> (HashMap<String, i32>, HashMap<String, Vec<MeetingBlock>>) {
    let mut start_times: HashMap<String, i32> = HashMap::new();
    let mut by_section: HashMap<String, Vec<MeetingBlock>> = HashMap::new();

    for row in meetings {
        let key = row.section_key();
        let start_min = match parse_time_to_minutes(&row.start_time) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let end_min = match parse_time_to_minutes(&row.end_time) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let day_code = row.day_code.trim().to_string();
        if day_code.is_empty() {
            continue;
        }

        start_times
            .entry(key.clone())
            .and_modify(|e| {
                if start_min < *e {
                    *e = start_min;
                }
            })
            .or_insert(start_min);

        by_section.entry(key).or_default().push(MeetingBlock {
            day_code,
            start_min,
            end_min,
        });
    }

    (start_times, by_section)
}

fn section_passes_earliest(section_id: &str, earliest: i32, starts: &HashMap<String, i32>) -> bool {
    if earliest == 0 {
        return true;
    }
    starts.get(section_id).copied().unwrap_or(0) >= earliest
}

fn stack_total_credits(stack: &[SectionRow]) -> f64 {
    stack.iter().map(|r| r.credits).sum()
}

fn passes_credit_bounds(total: f64, min_c: f64, max_c: f64) -> bool {
    const EPS: f64 = 1e-6;
    if min_c > EPS && total + EPS < min_c {
        return false;
    }
    if max_c > EPS && total > max_c + EPS {
        return false;
    }
    true
}

fn rotate_rows(rows: &[SectionRow], offset: usize) -> Vec<SectionRow> {
    if rows.is_empty() {
        return Vec::new();
    }
    let offset = offset % rows.len();
    if offset == 0 {
        return rows.to_vec();
    }
    let mut out = Vec::with_capacity(rows.len());
    out.extend_from_slice(&rows[offset..]);
    out.extend_from_slice(&rows[..offset]);
    out
}

fn unique_key_from_sections(sections: &[String]) -> String {
    let mut cp: Vec<&str> = sections.iter().map(|s| s.as_str()).collect();
    cp.sort_unstable();
    cp.join("|")
}

fn assign_schedule_ids(mut options: Vec<ScheduleOption>) -> Vec<ScheduleOption> {
    for (i, o) in options.iter_mut().enumerate() {
        o.id = format!("sched-{:03}", i + 1);
    }
    options
}

fn dfs_collect(
    pool: &[SectionRow],
    k: usize,
    max_candidates: usize,
    earliest_start: i32,
    max_per_subject: usize,
    starts: &HashMap<String, i32>,
    meetings_by_section: &HashMap<String, Vec<MeetingBlock>>,
    min_total_credits: f64,
    max_total_credits: f64,
    prereqs: &HashMap<String, Vec<Vec<String>>>,
    start: usize,
    stack: &mut Vec<SectionRow>,
    occ: &mut WeeklyOccupancy,
    results: &mut Vec<ScheduleOption>,
    seen: &mut HashSet<String>,
) {
    if results.len() >= max_candidates {
        return;
    }
    if stack.len() == k {
        let mut sections = Vec::with_capacity(k);
        for row in stack.iter() {
            sections.push(format!("{}-{}", row.course_code, row.section));
        }
        if !passes_credit_bounds(stack_total_credits(stack), min_total_credits, max_total_credits) {
            return;
        }
        let mut course_set: HashSet<String> = HashSet::with_capacity(k);
        for row in stack.iter() {
            course_set.insert(row.course_code.trim().to_uppercase());
        }
        if !schedule_satisfies_prereq_groups(&course_set, prereqs) {
            return;
        }
        let key = unique_key_from_sections(&sections);
        if seen.contains(&key) {
            return;
        }
        seen.insert(key);
        results.push(ScheduleOption {
            id: String::new(),
            sections,
            expected_utility: 0.0,
            stress_score: 0.0,
            academic_load_score: 0.0,
            lifestyle_penalty_score: 0.0,
        });
        return;
    }

    let mut used_course: HashSet<String> = HashSet::new();
    let mut subject_counts: HashMap<String, usize> = HashMap::new();
    for row in stack.iter() {
        used_course.insert(row.course_code.clone());
        *subject_counts.entry(row.subject_code.clone()).or_insert(0) += 1;
    }

    for i in start..pool.len() {
        let row = &pool[i];
        if used_course.contains(&row.course_code) {
            continue;
        }
        if subject_counts.get(&row.subject_code).copied().unwrap_or(0) >= max_per_subject {
            continue;
        }
        let section_id = format!("{}-{}", row.course_code, row.section);
        if !section_passes_earliest(&section_id, earliest_start, starts) {
            continue;
        }
        let new_blocks: &[MeetingBlock] = meetings_by_section
            .get(&section_id)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        if !occ.try_add_blocks(new_blocks) {
            continue;
        }
        stack.push(row.clone());
        dfs_collect(
            pool,
            k,
            max_candidates,
            earliest_start,
            max_per_subject,
            starts,
            meetings_by_section,
            min_total_credits,
            max_total_credits,
            prereqs,
            i + 1,
            stack,
            occ,
            results,
            seen,
        );
        stack.pop();
        occ.remove_blocks(new_blocks);
        if results.len() >= max_candidates {
            return;
        }
    }
}

fn candidates_for_seed(
    rows: &[SectionRow],
    seed: usize,
    k: usize,
    max_candidates: usize,
    earliest_start: i32,
    max_per_subject: usize,
    starts: &HashMap<String, i32>,
    meetings_by_section: &HashMap<String, Vec<MeetingBlock>>,
    min_total_credits: f64,
    max_total_credits: f64,
    prereqs: &HashMap<String, Vec<Vec<String>>>,
) -> Vec<ScheduleOption> {
    let pool = rotate_rows(rows, seed);
    let mut stack: Vec<SectionRow> = Vec::with_capacity(k);
    let mut occ = WeeklyOccupancy::new();
    let mut results: Vec<ScheduleOption> = Vec::with_capacity(max_candidates.min(256));
    let mut seen: HashSet<String> = HashSet::with_capacity(max_candidates);
    dfs_collect(
        &pool,
        k,
        max_candidates,
        earliest_start,
        max_per_subject,
        starts,
        meetings_by_section,
        min_total_credits,
        max_total_credits,
        prereqs,
        0,
        &mut stack,
        &mut occ,
        &mut results,
        &mut seen,
    );
    results
}

fn build_combinational_candidates(
    rows: &[SectionRow],
    k: usize,
    max_candidates: usize,
    earliest_start: i32,
    max_per_subject: usize,
    starts: &HashMap<String, i32>,
    meetings_by_section: &HashMap<String, Vec<MeetingBlock>>,
    min_total_credits: f64,
    max_total_credits: f64,
    prereqs: &HashMap<String, Vec<Vec<String>>>,
) -> Vec<ScheduleOption> {
    if rows.is_empty() || k == 0 {
        return Vec::new();
    }

    let seeds = [
        0usize,
        rows.len() / 7,
        rows.len() / 5,
        rows.len() / 3,
        rows.len() / 2,
    ];

    let batches: Vec<Vec<ScheduleOption>> = seeds
        .par_iter()
        .map(|&seed| {
            candidates_for_seed(
                rows,
                seed,
                k,
                max_candidates,
                earliest_start,
                max_per_subject,
                starts,
                meetings_by_section,
                min_total_credits,
                max_total_credits,
                prereqs,
            )
        })
        .collect();

    let mut merged: Vec<ScheduleOption> = Vec::with_capacity(max_candidates.min(256));
    let mut global_seen: HashSet<String> = HashSet::with_capacity(max_candidates);
    'outer: for batch in batches {
        for opt in batch {
            let key = unique_key_from_sections(&opt.sections);
            if global_seen.insert(key) {
                merged.push(opt);
                if merged.len() >= max_candidates {
                    break 'outer;
                }
            }
        }
    }

    assign_schedule_ids(merged)
}

fn filter_rows_by_subjects(
    rows: Vec<SectionRow>,
    whitelist: &HashSet<String>,
    blacklist: &HashSet<String>,
) -> Vec<SectionRow> {
    rows
        .into_iter()
        .filter(|row| {
            let subj = row.subject_code.to_uppercase();
            if !whitelist.is_empty() && !whitelist.contains(&subj) {
                return false;
            }
            if blacklist.contains(&subj) {
                return false;
            }
            true
        })
        .collect()
}

fn rebalance_rows_by_subject(rows: Vec<SectionRow>, max_rows: usize) -> Vec<SectionRow> {
    let mut by_subject: HashMap<String, Vec<SectionRow>> = HashMap::new();
    let mut subjects: Vec<String> = Vec::new();
    for row in rows {
        if !by_subject.contains_key(&row.subject_code) {
            subjects.push(row.subject_code.clone());
        }
        by_subject.entry(row.subject_code.clone()).or_default().push(row);
    }
    subjects.sort();

    let mut rebalanced = Vec::new();
    let mut idx = 0usize;
    while rebalanced.len() < max_rows {
        let mut progressed = false;
        for subj in &subjects {
            let entries = by_subject.get(subj).map(|v| v.as_slice()).unwrap_or(&[]);
            if idx < entries.len() {
                rebalanced.push(entries[idx].clone());
                progressed = true;
                if rebalanced.len() == max_rows {
                    break;
                }
            }
        }
        if !progressed {
            break;
        }
        idx += 1;
    }
    rebalanced
}

fn score_schedule(
    sections: &[String],
    by_section: &HashMap<String, Vec<MeetingBlock>>,
    weights: &ScoreWeights,
) -> (f64, f64, f64, f64) {
    const MIN_PER_WEEK_CAP: f64 = 1500.0;
    const EVENING_MIN_CAP: f64 = 360.0;
    const EARLY_MIN_CAP: f64 = 240.0;
    const BACK_TO_BACK_CAP: f64 = 8.0;
    const BUSY_DAY_MEET_CAP: f64 = 6.0;
    const EVENING_START_MIN: i32 = 17 * 60;
    const EARLY_END_MIN: i32 = 9 * 60;
    const BACK_TO_BACK_GAP_MIN: i32 = 12;

    let mut total_min = 0.0f64;
    let mut evening_min = 0.0f64;
    let mut early_min = 0.0f64;
    let mut missing_sections = 0i32;

    for sec in sections {
        let blocks = by_section.get(sec).map(|v| v.as_slice()).unwrap_or(&[]);
        if blocks.is_empty() {
            missing_sections += 1;
            continue;
        }
        for b in blocks {
            if b.end_min <= b.start_min {
                continue;
            }
            let dur = (b.end_min - b.start_min) as f64;
            total_min += dur;
            if b.start_min >= EVENING_START_MIN {
                evening_min += dur;
            }
            if b.start_min < EARLY_END_MIN {
                early_min += dur;
            }
        }
    }

    if missing_sections > 0 {
        let penalty = 0.08 * missing_sections as f64;
        total_min += penalty * MIN_PER_WEEK_CAP;
    }

    let mut by_day: HashMap<String, Vec<MeetingBlock>> = HashMap::new();
    for sec in sections {
        for b in by_section.get(sec).map(|v| v.as_slice()).unwrap_or(&[]) {
            if b.end_min <= b.start_min {
                continue;
            }
            by_day.entry(b.day_code.clone()).or_default().push(b.clone());
        }
    }

    let mut back_to_back = 0i32;
    let mut busy_day_max = 0usize;
    for day_blocks in by_day.values_mut() {
        if day_blocks.len() > busy_day_max {
            busy_day_max = day_blocks.len();
        }
        day_blocks.sort_by_key(|b| b.start_min);
        for i in 1..day_blocks.len() {
            let gap = day_blocks[i].start_min - day_blocks[i - 1].end_min;
            if gap >= 0 && gap < BACK_TO_BACK_GAP_MIN {
                back_to_back += 1;
            }
        }
    }

    let n_weekly = (total_min / MIN_PER_WEEK_CAP).min(1.0);
    let n_evening = (evening_min / EVENING_MIN_CAP).min(1.0);
    let n_early = (early_min / EARLY_MIN_CAP).min(1.0);
    let n_back = ((back_to_back as f64) / BACK_TO_BACK_CAP).min(1.0);
    let n_busy = ((busy_day_max as f64) / BUSY_DAY_MEET_CAP).min(1.0);

    let academic_raw = 0.65 * n_weekly + 0.20 * n_back + 0.15 * n_busy;
    let lifestyle_raw = 0.65 * n_evening + 0.35 * n_early;
    let academic = round_float(academic_raw.min(1.0), 3);
    let lifestyle = round_float(lifestyle_raw.min(1.0), 3);

    let raw = weights.weekly * n_weekly
        + weights.evening * n_evening
        + weights.early * n_early
        + weights.back_to_back * n_back
        + weights.busy_day * n_busy;
    let stress = round_float(raw.min(1.0), 3);
    let utility = round_float((1.0 - stress).max(0.0), 3);

    (stress, utility, academic, lifestyle)
}

fn apply_schedule_scores(options: &mut [ScheduleOption], by_section: &HashMap<String, Vec<MeetingBlock>>, weights: &ScoreWeights) {
    options.par_iter_mut().for_each(|opt| {
        let (s, u, a, l) = score_schedule(&opt.sections, by_section, weights);
        opt.stress_score = s;
        opt.expected_utility = u;
        opt.academic_load_score = a;
        opt.lifestyle_penalty_score = l;
    });
}

fn dominates(a: &ScheduleOption, b: &ScheduleOption) -> bool {
    let ge_a = a.academic_load_score <= b.academic_load_score;
    let ge_l = a.lifestyle_penalty_score <= b.lifestyle_penalty_score;
    let strict = a.academic_load_score < b.academic_load_score || a.lifestyle_penalty_score < b.lifestyle_penalty_score;
    ge_a && ge_l && strict
}

fn pareto_frontier(options: &[ScheduleOption]) -> Vec<ScheduleOption> {
    let mut frontier = Vec::new();
    for (i, candidate) in options.iter().enumerate() {
        let mut dominated = false;
        for (j, other) in options.iter().enumerate() {
            if i == j {
                continue;
            }
            if dominates(other, candidate) {
                dominated = true;
                break;
            }
        }
        if !dominated {
            frontier.push(candidate.clone());
        }
    }
    frontier
}

fn epsilon_frontier(options: &[ScheduleOption], eps: f64) -> Vec<ScheduleOption> {
    if options.is_empty() {
        return Vec::new();
    }
    let mut best_academic = options[0].academic_load_score;
    let mut best_lifestyle = options[0].lifestyle_penalty_score;
    for opt in options.iter() {
        if opt.academic_load_score < best_academic {
            best_academic = opt.academic_load_score;
        }
        if opt.lifestyle_penalty_score < best_lifestyle {
            best_lifestyle = opt.lifestyle_penalty_score;
        }
    }
    options
        .iter()
        .filter(|o| {
            o.academic_load_score <= best_academic + eps && o.lifestyle_penalty_score <= best_lifestyle + eps
        })
        .cloned()
        .collect()
}

fn min_usize(a: usize, b: usize) -> usize {
    if a < b { a } else { b }
}

/// Run full optimization pipeline. Conflict detection uses the same meeting rows as `detect_conflicts`.
pub fn run_optimize(
    sections_path: &Path,
    meetings_path: &Path,
    params: OptimizeParams,
) -> Result<(Vec<ScheduleOption>, ScoreWeights, Option<String>), String> {
    let effective = normalize_weights(params.weights.clone());

    let rows = load_sections_csv(sections_path, 0)?;
    let whitelist: HashSet<String> = params
        .subject_whitelist
        .iter()
        .map(|s| s.trim().to_uppercase())
        .filter(|s| !s.is_empty())
        .collect();
    let blacklist: HashSet<String> = params
        .subject_blacklist
        .iter()
        .map(|s| s.trim().to_uppercase())
        .filter(|s| !s.is_empty())
        .collect();

    let mut rows = filter_rows_by_subjects(rows, &whitelist, &blacklist);
    if rows.is_empty() {
        return Ok((Vec::new(), effective, Some("no_sections_after_subject_filters".into())));
    }
    rows = rebalance_rows_by_subject(rows, 180);

    let meetings = load_meetings_from_csv(meetings_path)?;
    let (start_times, meetings_by_section) = build_meeting_blocks_and_starts(&meetings);

    let k = if params.k == 0 {
        4usize
    } else {
        params.k.clamp(2, 6) as usize
    };
    let max_candidates = if params.max_candidates > 0 {
        params.max_candidates as usize
    } else {
        2000
    };
    let max_per_subject = if params.max_per_subject == 0 {
        1usize
    } else {
        params.max_per_subject.clamp(1, 3) as usize
    };
    let earliest = params.earliest_start_minutes.max(0);

    let min_cr = params.min_total_credits.max(0.0);
    let max_cr = params.max_total_credits.max(0.0);

    let prereq_map: HashMap<String, Vec<Vec<String>>> = match companion_courses_csv(sections_path) {
        Some(p) => load_prereq_clauses_map(&p)?,
        None => HashMap::new(),
    };

    let mut candidates = build_combinational_candidates(
        &rows,
        k,
        max_candidates,
        earliest,
        max_per_subject,
        &start_times,
        &meetings_by_section,
        min_cr,
        max_cr,
        &prereq_map,
    );

    if candidates.is_empty() {
        return Ok((Vec::new(), effective, Some("no_candidates_generated".into())));
    }

    apply_schedule_scores(&mut candidates, &meetings_by_section, &effective);

    candidates.sort_by(|a, b| {
        b.expected_utility
            .partial_cmp(&a.expected_utility)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.stress_score.partial_cmp(&b.stress_score).unwrap_or(std::cmp::Ordering::Equal))
    });

    let max_results = if params.max_results == 0 {
        10usize
    } else {
        params.max_results.clamp(1, 25) as usize
    };

    let conflict_free: Vec<bool> = candidates
        .par_iter()
        .map(|opt| detect_conflicts(&meetings, &opt.sections).is_empty())
        .collect();

    let mut filtered: Vec<ScheduleOption> = Vec::new();
    for (opt, ok) in candidates.iter().zip(conflict_free.iter()) {
        if *ok {
            filtered.push(opt.clone());
            if !params.pareto && filtered.len() == max_results {
                break;
            }
        }
    }

    let mut reason: Option<String> = None;

    if params.pareto {
        let mode = params.pareto_mode.to_lowercase();
        let eps = if params.pareto_epsilon > 0.0 {
            params.pareto_epsilon
        } else {
            0.03
        };
        let mut conflict_safe = if mode == "epsilon" {
            epsilon_frontier(&filtered, eps)
        } else {
            let mut f = pareto_frontier(&filtered);
            if f.len() < min_usize(3, max_results) {
                f = epsilon_frontier(&filtered, eps);
            }
            f
        };
        conflict_safe.sort_by(|a, b| {
            b.expected_utility
                .partial_cmp(&a.expected_utility)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.stress_score.partial_cmp(&b.stress_score).unwrap_or(std::cmp::Ordering::Equal))
        });
        if conflict_safe.len() > max_results {
            conflict_safe.truncate(max_results);
        }
        filtered = conflict_safe;
    } else if filtered.len() > max_results {
        filtered.truncate(max_results);
    }

    if filtered.is_empty() && reason.is_none() {
        reason = Some("no_conflict_free_options_found".into());
    }

    Ok((filtered, effective, reason))
}

#[cfg(test)]
mod credit_bounds_tests {
    use super::*;

    #[test]
    fn passes_credit_bounds_respects_min_and_max() {
        assert!(passes_credit_bounds(6.0, 6.0, 0.0));
        assert!(!passes_credit_bounds(5.99, 6.0, 0.0));
        assert!(passes_credit_bounds(6.0, 0.0, 6.0));
        assert!(!passes_credit_bounds(6.01, 0.0, 6.0));
        assert!(passes_credit_bounds(5.0, 0.0, 0.0));
    }

    #[test]
    fn stack_total_credits_sums_rows() {
        let stack = vec![
            SectionRow {
                subject_code: "A".into(),
                course_code: "A1".into(),
                section: "01".into(),
                credits: 3.0,
            },
            SectionRow {
                subject_code: "B".into(),
                course_code: "B1".into(),
                section: "01".into(),
                credits: 1.5,
            },
        ];
        assert!((stack_total_credits(&stack) - 4.5).abs() < 1e-9);
    }

    #[test]
    fn schedule_satisfies_prereq_groups_and_or() {
        let mut m: HashMap<String, Vec<Vec<String>>> = HashMap::new();
        // COMP200 needs COMP112 (singleton OR-group)
        m.insert("COMP200".into(), vec![vec!["COMP112".into()]]);
        let ok: HashSet<String> = ["COMP200", "COMP112"].iter().map(|s| s.to_string()).collect();
        assert!(schedule_satisfies_prereq_groups(&ok, &m));
        let bad: HashSet<String> = ["COMP200"].iter().map(|s| s.to_string()).collect();
        assert!(!schedule_satisfies_prereq_groups(&bad, &m));
        // ECON110 needs any of MATH120 or MATH121
        m.insert(
            "ECON110".into(),
            vec![vec!["MATH120".into(), "MATH121".into()]],
        );
        let ok_or: HashSet<String> = ["ECON110", "MATH121"].iter().map(|s| s.to_string()).collect();
        assert!(schedule_satisfies_prereq_groups(&ok_or, &m));
        let bad_or: HashSet<String> = ["ECON110", "COMP112"].iter().map(|s| s.to_string()).collect();
        assert!(!schedule_satisfies_prereq_groups(&bad_or, &m));
    }

    #[test]
    fn transitive_prereq_chain_requires_all_steps_in_schedule() {
        let mut m: HashMap<String, Vec<Vec<String>>> = HashMap::new();
        m.insert("COMP300".into(), vec![vec!["COMP200".into()]]);
        m.insert("COMP200".into(), vec![vec!["COMP112".into()]]);
        let full: HashSet<String> = ["COMP300", "COMP200", "COMP112"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(schedule_satisfies_prereq_groups(&full, &m));
        let skip_middle: HashSet<String> = ["COMP300", "COMP112"].iter().map(|s| s.to_string()).collect();
        assert!(!schedule_satisfies_prereq_groups(&skip_middle, &m));
        let skip_base: HashSet<String> = ["COMP300", "COMP200"].iter().map(|s| s.to_string()).collect();
        assert!(!schedule_satisfies_prereq_groups(&skip_base, &m));
    }

    #[test]
    fn or_alternative_must_satisfy_its_own_prereqs() {
        let mut m: HashMap<String, Vec<Vec<String>>> = HashMap::new();
        m.insert(
            "ECON110".into(),
            vec![vec!["MATH120".into(), "MATH121".into()]],
        );
        m.insert("MATH121".into(), vec![vec!["MATH120".into()]]);
        // MATH121 alone is not enough; need MATH120 for MATH121's prereq
        let bad: HashSet<String> = ["ECON110", "MATH121"].iter().map(|s| s.to_string()).collect();
        assert!(!schedule_satisfies_prereq_groups(&bad, &m));
        let ok: HashSet<String> = ["ECON110", "MATH120", "MATH121"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(schedule_satisfies_prereq_groups(&ok, &m));
    }

    #[test]
    fn prereq_cycle_in_schedule_fails() {
        let mut m: HashMap<String, Vec<Vec<String>>> = HashMap::new();
        m.insert("A".into(), vec![vec!["B".into()]]);
        m.insert("B".into(), vec![vec!["A".into()]]);
        let sel: HashSet<String> = ["A", "B"].iter().map(|s| s.to_string()).collect();
        assert!(!schedule_satisfies_prereq_groups(&sel, &m));
    }
}

#[cfg(test)]
mod interval_prune_tests {
    use super::*;
    use crate::weekly_bitmap::WeeklyOccupancy;

    fn blk(day: &str, s: i32, e: i32) -> MeetingBlock {
        MeetingBlock {
            day_code: day.into(),
            start_min: s,
            end_min: e,
        }
    }

    #[test]
    fn weekly_bitmap_matches_half_open_overlap_semantics() {
        let mut occ = WeeklyOccupancy::new();
        assert!(occ.try_add_blocks(&[blk("T", 530, 610)]));
        assert!(!occ.try_add_blocks(&[blk("T", 570, 620)])); // overlap
        occ.remove_blocks(&[blk("T", 530, 610)]);
        assert!(occ.try_add_blocks(&[blk("T", 480, 540)]));
        assert!(occ.try_add_blocks(&[blk("T", 540, 600)])); // adjacent, no overlap
        occ.remove_blocks(&[blk("T", 480, 540)]);
        occ.remove_blocks(&[blk("T", 540, 600)]);
        assert!(occ.try_add_blocks(&[blk("T", 480, 540)]));
        assert!(occ.try_add_blocks(&[blk("T", 600, 660)])); // gap, no overlap
    }

    #[test]
    fn dfs_prunes_time_overlapping_sections() {
        let rows = vec![
            SectionRow {
                subject_code: "COMP".into(),
                course_code: "COMP100".into(),
                section: "01".into(),
                credits: 1.0,
            },
            SectionRow {
                subject_code: "COMP".into(),
                course_code: "COMP101".into(),
                section: "01".into(),
                credits: 1.0,
            },
            SectionRow {
                subject_code: "MATH".into(),
                course_code: "MATH121".into(),
                section: "01".into(),
                credits: 1.0,
            },
        ];
        let mut meetings_by_section: HashMap<String, Vec<MeetingBlock>> = HashMap::new();
        meetings_by_section.insert(
            "COMP100-01".into(),
            vec![blk("T", 530, 610)],
        );
        meetings_by_section.insert(
            "COMP101-01".into(),
            vec![blk("T", 570, 620)],
        );
        meetings_by_section.insert("MATH121-01".into(), vec![blk("W", 600, 660)]);
        let starts: HashMap<String, i32> = [
            ("COMP100-01".into(), 530),
            ("COMP101-01".into(), 570),
            ("MATH121-01".into(), 600),
        ]
        .into_iter()
        .collect();
        let prereqs: HashMap<String, Vec<Vec<String>>> = HashMap::new();
        let opts = build_combinational_candidates(
            &rows,
            2,
            50,
            0,
            2,
            &starts,
            &meetings_by_section,
            0.0,
            0.0,
            &prereqs,
        );
        let keys: Vec<String> = opts.iter().map(|o| o.sections.join("|")).collect();
        assert!(keys.iter().any(|k| k.contains("COMP100") && k.contains("MATH121")));
        assert!(keys.iter().any(|k| k.contains("COMP101") && k.contains("MATH121")));
        assert!(!keys.iter().any(|k| k.contains("COMP100") && k.contains("COMP101")));
    }
}
