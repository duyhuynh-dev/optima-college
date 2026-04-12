//! Per-day minute bitmaps (1440 bits per day) for fast DFS time pruning.
//! Semantics: half-open minutes `[start_min, end_min)`; overlap iff `s1 < e2 && s2 < e1`.

use std::collections::HashMap;

/// One class session block for a section (same shape as rows built from meetings CSV).
#[derive(Debug, Clone)]
pub(crate) struct MeetingBlock {
    pub day_code: String,
    pub start_min: i32,
    pub end_min: i32,
}

pub(crate) const MINUTES_PER_DAY: i32 = 24 * 60;
pub(crate) const DAY_WORDS: usize = (MINUTES_PER_DAY as usize + 63) / 64;

#[inline]
fn clamp_range(s: i32, e: i32) -> Option<(usize, usize)> {
    let s0 = s.clamp(0, MINUTES_PER_DAY) as usize;
    let e0 = e.clamp(0, MINUTES_PER_DAY) as usize;
    if e0 > s0 {
        Some((s0, e0))
    } else {
        None
    }
}

fn word_mask(bi: usize, width: usize) -> u64 {
    debug_assert!(bi < 64);
    debug_assert!(width <= 64 - bi);
    if width == 0 {
        return 0;
    }
    if width == 64 && bi == 0 {
        return u64::MAX;
    }
    ((1u64 << width) - 1) << bi
}

fn range_overlaps(words: &[u64; DAY_WORDS], s: i32, e: i32) -> bool {
    let Some((s0, e0)) = clamp_range(s, e) else {
        return false;
    };
    let mut pos = s0;
    while pos < e0 {
        let wi = pos / 64;
        let bi = pos % 64;
        let end_here = e0.min((wi + 1) * 64);
        let width = end_here - pos;
        let mask = word_mask(bi, width);
        if words[wi] & mask != 0 {
            return true;
        }
        pos = end_here;
    }
    false
}

fn range_or(words: &mut [u64; DAY_WORDS], s: i32, e: i32) {
    let Some((s0, e0)) = clamp_range(s, e) else {
        return;
    };
    let mut pos = s0;
    while pos < e0 {
        let wi = pos / 64;
        let bi = pos % 64;
        let end_here = e0.min((wi + 1) * 64);
        let width = end_here - pos;
        words[wi] |= word_mask(bi, width);
        pos = end_here;
    }
}

fn range_clear(words: &mut [u64; DAY_WORDS], s: i32, e: i32) {
    let Some((s0, e0)) = clamp_range(s, e) else {
        return;
    };
    let mut pos = s0;
    while pos < e0 {
        let wi = pos / 64;
        let bi = pos % 64;
        let end_here = e0.min((wi + 1) * 64);
        let width = end_here - pos;
        words[wi] &= !word_mask(bi, width);
        pos = end_here;
    }
}

#[derive(Clone, Default)]
pub(crate) struct WeeklyOccupancy {
    days: HashMap<String, [u64; DAY_WORDS]>,
}

impl WeeklyOccupancy {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Returns `false` if any new block overlaps an occupied minute on that day.
    pub(crate) fn try_add_blocks(&mut self, blocks: &[MeetingBlock]) -> bool {
        for b in blocks {
            let w = self
                .days
                .entry(b.day_code.clone())
                .or_insert([0u64; DAY_WORDS]);
            if range_overlaps(w, b.start_min, b.end_min) {
                return false;
            }
        }
        for b in blocks {
            let w = self.days.get_mut(&b.day_code).expect("entry exists");
            range_or(w, b.start_min, b.end_min);
        }
        true
    }

    pub(crate) fn remove_blocks(&mut self, blocks: &[MeetingBlock]) {
        for b in blocks {
            if let Some(w) = self.days.get_mut(&b.day_code) {
                range_clear(w, b.start_min, b.end_min);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mb(day: &str, s: i32, e: i32) -> MeetingBlock {
        MeetingBlock {
            day_code: day.into(),
            start_min: s,
            end_min: e,
        }
    }

    #[test]
    fn try_add_detects_overlap_same_day() {
        let mut occ = WeeklyOccupancy::new();
        assert!(occ.try_add_blocks(&[mb("T", 530, 610)]));
        assert!(!occ.try_add_blocks(&[mb("T", 570, 620)]));
    }

    #[test]
    fn remove_blocks_restores_try_add() {
        let mut occ = WeeklyOccupancy::new();
        let a = vec![mb("T", 530, 610)];
        assert!(occ.try_add_blocks(&a));
        occ.remove_blocks(&a);
        assert!(occ.try_add_blocks(&[mb("T", 570, 620)]));
    }
}
