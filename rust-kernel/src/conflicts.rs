use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Clone)]
pub struct MeetingRecord {
    pub term: String,
    pub term_label: String,
    pub subject_code: String,
    pub course_code: String,
    pub course_ref: String,
    pub section: String,
    pub day_code: String,
    pub day_name: String,
    pub start_time: String,
    pub end_time: String,
    pub source_url: String,
}

impl MeetingRecord {
    pub fn section_key(&self) -> String {
        format!("{}-{}", self.course_code, self.section)
    }
}

#[derive(Debug, Serialize, Clone)]
pub struct ConflictPair {
    pub day_code: String,
    pub day_name: String,
    pub left_section: String,
    pub right_section: String,
    pub left_start: String,
    pub left_end: String,
    pub right_start: String,
    pub right_end: String,
}

pub fn load_meetings_from_csv(path: &Path) -> Result<Vec<MeetingRecord>, String> {
    let mut reader = csv::Reader::from_path(path)
        .map_err(|err| format!("failed to open meetings csv {}: {err}", path.display()))?;

    let mut rows = Vec::new();
    for row in reader.deserialize() {
        let parsed: MeetingRecord = row.map_err(|err| format!("failed to parse csv row: {err}"))?;
        rows.push(parsed);
    }
    Ok(rows)
}

/// Parsed meeting on one day for pairwise overlap checks (same rule as DFS pruning: `s1 < e2 && s2 < e1`).
struct DayMeeting<'a> {
    row: &'a MeetingRecord,
    start_min: i32,
    end_min: i32,
}

pub fn detect_conflicts(meetings: &[MeetingRecord], selected_sections: &[String]) -> Vec<ConflictPair> {
    let selected: HashSet<String> = selected_sections.iter().cloned().collect();
    let mut by_day: HashMap<String, Vec<DayMeeting<'_>>> = HashMap::new();

    for row in meetings {
        if !selected.contains(&row.section_key()) {
            continue;
        }
        let (start_min, end_min) = match (
            parse_time_to_minutes(&row.start_time),
            parse_time_to_minutes(&row.end_time),
        ) {
            (Ok(s), Ok(e)) => (s, e),
            _ => continue,
        };
        if end_min <= start_min {
            continue;
        }
        by_day.entry(row.day_code.clone()).or_default().push(DayMeeting {
            row,
            start_min,
            end_min,
        });
    }

    let mut conflicts = Vec::new();

    for day_meetings in by_day.values() {
        let n = day_meetings.len();
        for i in 0..n {
            for j in (i + 1)..n {
                let a = &day_meetings[i];
                let b = &day_meetings[j];
                let key_a = a.row.section_key();
                let key_b = b.row.section_key();
                if key_a == key_b {
                    continue;
                }
                if a.start_min < b.end_min && b.start_min < a.end_min {
                    conflicts.push(ConflictPair {
                        day_code: a.row.day_code.clone(),
                        day_name: a.row.day_name.clone(),
                        left_section: key_a,
                        right_section: key_b,
                        left_start: a.row.start_time.clone(),
                        left_end: a.row.end_time.clone(),
                        right_start: b.row.start_time.clone(),
                        right_end: b.row.end_time.clone(),
                    });
                }
            }
        }
    }

    conflicts
}

pub fn parse_time_to_minutes(value: &str) -> Result<i32, String> {
    let value = value.trim().to_uppercase();
    let suffix = if value.ends_with("AM") {
        "AM"
    } else if value.ends_with("PM") {
        "PM"
    } else {
        return Err(format!("invalid time suffix: {value}"));
    };

    let hm = value
        .strip_suffix(suffix)
        .ok_or_else(|| format!("invalid time value: {value}"))?
        .trim();

    let mut parts = hm.split(':');
    let hour = parts
        .next()
        .ok_or_else(|| format!("missing hour in {value}"))?
        .parse::<i32>()
        .map_err(|_| format!("invalid hour in {value}"))?;
    let minute = parts
        .next()
        .ok_or_else(|| format!("missing minute in {value}"))?
        .parse::<i32>()
        .map_err(|_| format!("invalid minute in {value}"))?;

    if !(1..=12).contains(&hour) || !(0..=59).contains(&minute) {
        return Err(format!("time out of range: {value}"));
    }

    let mut hour_24 = hour % 12;
    if suffix == "PM" {
        hour_24 += 12;
    }

    Ok(hour_24 * 60 + minute)
}

#[cfg(test)]
mod tests {
    use super::{detect_conflicts, parse_time_to_minutes, MeetingRecord};

    fn record(section: &str, day_code: &str, day_name: &str, start: &str, end: &str) -> MeetingRecord {
        record_course("COMP112", section, day_code, day_name, start, end)
    }

    fn record_course(
        course: &str,
        section: &str,
        day_code: &str,
        day_name: &str,
        start: &str,
        end: &str,
    ) -> MeetingRecord {
        MeetingRecord {
            term: "1269".to_string(),
            term_label: "Fall 2026".to_string(),
            subject_code: "COMP".to_string(),
            course_code: course.to_string(),
            course_ref: "003328".to_string(),
            section: section.to_string(),
            day_code: day_code.to_string(),
            day_name: day_name.to_string(),
            start_time: start.to_string(),
            end_time: end.to_string(),
            source_url: "test".to_string(),
        }
    }

    #[test]
    fn parses_time_to_minutes() {
        assert_eq!(parse_time_to_minutes("08:50AM").unwrap(), 8 * 60 + 50);
        assert_eq!(parse_time_to_minutes("12:10PM").unwrap(), 12 * 60 + 10);
        assert_eq!(parse_time_to_minutes("12:00AM").unwrap(), 0);
    }

    #[test]
    fn detects_overlap_for_selected_sections() {
        let meetings = vec![
            record("01", "T", "Tuesday", "08:50AM", "10:10AM"),
            record("02", "T", "Tuesday", "09:30AM", "10:20AM"),
            record("03", "R", "Thursday", "10:20AM", "11:40AM"),
        ];

        let selected = vec!["COMP112-01".to_string(), "COMP112-02".to_string()];
        let conflicts = detect_conflicts(&meetings, &selected);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].day_code, "T");
    }

    /// Three courses same day: short middle meeting overlaps only the first long block; third overlaps
    /// the first but not the middle. A sweep that only compares consecutive starts would miss the 1st–3rd pair.
    #[test]
    fn detects_non_adjacent_overlaps_on_same_day() {
        let meetings = vec![
            record_course("COMP100", "01", "T", "Tuesday", "08:00AM", "10:00AM"),
            record_course("COMP200", "01", "T", "Tuesday", "08:30AM", "09:00AM"),
            record_course("COMP300", "01", "T", "Tuesday", "09:30AM", "11:00AM"),
        ];
        let selected = vec![
            "COMP100-01".to_string(),
            "COMP200-01".to_string(),
            "COMP300-01".to_string(),
        ];
        let conflicts = detect_conflicts(&meetings, &selected);
        assert_eq!(
            conflicts.len(),
            2,
            "COMP100–COMP200 and COMP100–COMP300 overlap; COMP200–COMP300 do not"
        );
        let mut pairs: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
        for c in &conflicts {
            let a = c.left_section.clone();
            let b = c.right_section.clone();
            let p = if a <= b { (a, b) } else { (b, a) };
            pairs.insert(p);
        }
        assert!(pairs.contains(&(
            "COMP100-01".to_string(),
            "COMP200-01".to_string()
        )));
        assert!(pairs.contains(&(
            "COMP100-01".to_string(),
            "COMP300-01".to_string()
        )));
    }
}
