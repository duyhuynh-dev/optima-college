//! Local schedule optimization: candidate generation, scoring (aligned with Go orchestrator), and conflict filtering.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Deserialize;

use crate::conflicts::{detect_conflicts, load_meetings_from_csv, parse_time_to_minutes, MeetingRecord};

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
}

#[derive(Debug, Clone, Deserialize)]
struct SectionRow {
    subject_code: String,
    course_code: String,
    section: String,
}

#[derive(Clone)]
struct MeetingBlock {
    day_code: String,
    start_min: i32,
    end_min: i32,
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

fn build_combinational_candidates(
    rows: &[SectionRow],
    k: usize,
    max_candidates: usize,
    earliest_start: i32,
    max_per_subject: usize,
    starts: &HashMap<String, i32>,
) -> Vec<ScheduleOption> {
    if rows.is_empty() || k == 0 {
        return Vec::new();
    }

    let mut results: Vec<ScheduleOption> = Vec::with_capacity(max_candidates.min(256));
    let mut seen: HashSet<String> = HashSet::with_capacity(max_candidates);

    let seeds = [
        0usize,
        rows.len() / 7,
        rows.len() / 5,
        rows.len() / 3,
        rows.len() / 2,
    ];

    for &seed in &seeds {
        let pool = rotate_rows(rows, seed);
        let mut stack: Vec<SectionRow> = Vec::with_capacity(k);

        fn dfs(
            pool: &[SectionRow],
            k: usize,
            max_candidates: usize,
            earliest_start: i32,
            max_per_subject: usize,
            starts: &HashMap<String, i32>,
            start: usize,
            stack: &mut Vec<SectionRow>,
            results: &mut Vec<ScheduleOption>,
            seen: &mut HashSet<String>,
        ) {
            if results.len() >= max_candidates {
                return;
            }
            if stack.len() == k {
                let mut sections = Vec::with_capacity(k);
                for row in stack.iter() {
                    let section_id = format!("{}-{}", row.course_code, row.section);
                    if !section_passes_earliest(&section_id, earliest_start, starts) {
                        return;
                    }
                    sections.push(section_id);
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
                stack.push(row.clone());
                dfs(
                    pool,
                    k,
                    max_candidates,
                    earliest_start,
                    max_per_subject,
                    starts,
                    i + 1,
                    stack,
                    results,
                    seen,
                );
                stack.pop();
                if results.len() >= max_candidates {
                    return;
                }
            }
        }

        dfs(
            &pool,
            k,
            max_candidates,
            earliest_start,
            max_per_subject,
            starts,
            0,
            &mut stack,
            &mut results,
            &mut seen,
        );

        if results.len() >= max_candidates {
            break;
        }
    }

    assign_schedule_ids(results)
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
    for opt in options.iter_mut() {
        let (s, u, a, l) = score_schedule(&opt.sections, by_section, weights);
        opt.stress_score = s;
        opt.expected_utility = u;
        opt.academic_load_score = a;
        opt.lifestyle_penalty_score = l;
    }
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

    let mut candidates = build_combinational_candidates(
        &rows,
        k,
        max_candidates,
        earliest,
        max_per_subject,
        &start_times,
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

    let mut filtered: Vec<ScheduleOption> = Vec::new();
    for opt in candidates.iter() {
        let conflicts = detect_conflicts(&meetings, &opt.sections);
        if conflicts.is_empty() {
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
