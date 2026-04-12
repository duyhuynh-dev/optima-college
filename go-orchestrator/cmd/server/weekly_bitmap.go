package main

const minutesPerDay = 24 * 60
const dayWords = (minutesPerDay + 63) / 64 // 23

func clampInt(v, lo, hi int) int {
	if v < lo {
		return lo
	}
	if v > hi {
		return hi
	}
	return v
}

func clampRangeMinutes(s, e int) (int, int, bool) {
	s0 := clampInt(s, 0, minutesPerDay)
	e0 := clampInt(e, 0, minutesPerDay)
	if e0 > s0 {
		return s0, e0, true
	}
	return 0, 0, false
}

func wordMaskU64(bi, width int) uint64 {
	if width <= 0 {
		return 0
	}
	if width == 64 && bi == 0 {
		return ^uint64(0)
	}
	return ((uint64(1) << uint(width)) - 1) << uint(bi)
}

func rangeOverlapsWords(words *[dayWords]uint64, s, e int) bool {
	s0, e0, ok := clampRangeMinutes(s, e)
	if !ok {
		return false
	}
	pos := s0
	for pos < e0 {
		wi := pos / 64
		bi := pos % 64
		endHere := e0
		if next := (wi + 1) * 64; next < endHere {
			endHere = next
		}
		width := endHere - pos
		mask := wordMaskU64(bi, width)
		if words[wi]&mask != 0 {
			return true
		}
		pos = endHere
	}
	return false
}

func rangeOrWords(words *[dayWords]uint64, s, e int) {
	s0, e0, ok := clampRangeMinutes(s, e)
	if !ok {
		return
	}
	pos := s0
	for pos < e0 {
		wi := pos / 64
		bi := pos % 64
		endHere := e0
		if next := (wi + 1) * 64; next < endHere {
			endHere = next
		}
		width := endHere - pos
		words[wi] |= wordMaskU64(bi, width)
		pos = endHere
	}
}

func rangeClearWords(words *[dayWords]uint64, s, e int) {
	s0, e0, ok := clampRangeMinutes(s, e)
	if !ok {
		return
	}
	pos := s0
	for pos < e0 {
		wi := pos / 64
		bi := pos % 64
		endHere := e0
		if next := (wi + 1) * 64; next < endHere {
			endHere = next
		}
		width := endHere - pos
		words[wi] &^= wordMaskU64(bi, width)
		pos = endHere
	}
}

// weeklyTimeBitmap tracks occupied minutes per day_code (same semantics as Rust DFS pruning).
type weeklyTimeBitmap struct {
	days map[string][dayWords]uint64
}

func newWeeklyTimeBitmap() *weeklyTimeBitmap {
	return &weeklyTimeBitmap{days: make(map[string][dayWords]uint64)}
}

func (w *weeklyTimeBitmap) tryAddBlocks(blocks []meetingBlock) bool {
	for _, b := range blocks {
		arr := w.days[b.DayCode]
		if rangeOverlapsWords(&arr, b.StartMin, b.EndMin) {
			return false
		}
	}
	for _, b := range blocks {
		arr := w.days[b.DayCode]
		rangeOrWords(&arr, b.StartMin, b.EndMin)
		w.days[b.DayCode] = arr
	}
	return true
}

func (w *weeklyTimeBitmap) removeBlocks(blocks []meetingBlock) {
	for _, b := range blocks {
		arr := w.days[b.DayCode]
		rangeClearWords(&arr, b.StartMin, b.EndMin)
		w.days[b.DayCode] = arr
	}
}
