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

pub fn detect_conflicts(meetings: &[MeetingRecord], selected_sections: &[String]) -> Vec<ConflictPair> {
    let selected: HashSet<String> = selected_sections.iter().cloned().collect();
    let mut by_day: HashMap<String, Vec<&MeetingRecord>> = HashMap::new();

    for row in meetings {
        if selected.contains(&row.section_key()) {
            by_day.entry(row.day_code.clone()).or_default().push(row);
        }
    }

    let mut conflicts = Vec::new();

    for day_rows in by_day.values_mut() {
        day_rows.sort_by_key(|row| parse_time_to_minutes(&row.start_time).unwrap_or(i32::MAX));

        for i in 1..day_rows.len() {
            let prev = day_rows[i - 1];
            let current = day_rows[i];

            let prev_end = parse_time_to_minutes(&prev.end_time).unwrap_or(-1);
            let current_start = parse_time_to_minutes(&current.start_time).unwrap_or(-1);
            let prev_section = prev.section_key();
            let current_section = current.section_key();

            if prev_section != current_section && current_start < prev_end {
                conflicts.push(ConflictPair {
                    day_code: current.day_code.clone(),
                    day_name: current.day_name.clone(),
                    left_section: prev_section,
                    right_section: current_section,
                    left_start: prev.start_time.clone(),
                    left_end: prev.end_time.clone(),
                    right_start: current.start_time.clone(),
                    right_end: current.end_time.clone(),
                });
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
        MeetingRecord {
            term: "1269".to_string(),
            term_label: "Fall 2026".to_string(),
            subject_code: "COMP".to_string(),
            course_code: "COMP112".to_string(),
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
}
