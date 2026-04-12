package main

import (
	"context"
	"encoding/csv"
	"encoding/json"
	"fmt"
	"io"
	"log"
	"math"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"time"

	optimav1 "optima/go-orchestrator/internal/gen/optima/v1"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
)

type ScheduleOption struct {
	ID                    string          `json:"id"`
	Sections              []string        `json:"sections"`
	ExpectedUtility       float64         `json:"expected_utility"`
	StressScore           float64         `json:"stress_score"`
	AcademicLoadScore     float64         `json:"academic_load_score"`
	LifestylePenaltyScore float64         `json:"lifestyle_penalty_score"`
	ScoreBreakdown        *ScoreBreakdown `json:"score_breakdown,omitempty"`
}

// ScoreBreakdown is returned when debug=1; shows raw inputs and normalized terms used in stress.
type ScoreBreakdown struct {
	TotalMinutes    int     `json:"total_minutes"`
	EveningMinutes  int     `json:"evening_minutes"`
	EarlyMinutes    int     `json:"early_minutes"`
	BackToBackPairs int     `json:"back_to_back_pairs"`
	BusyDayMax      int     `json:"busy_day_max"`
	MissingSections int     `json:"missing_sections"`
	NormWeekly      float64 `json:"norm_weekly"`
	NormEvening     float64 `json:"norm_evening"`
	NormEarly       float64 `json:"norm_early"`
	NormBackToBack  float64 `json:"norm_back_to_back"`
	NormBusyDay     float64 `json:"norm_busy_day"`
	AcademicLoad    float64 `json:"academic_load"`
	LifestylePenalty float64 `json:"lifestyle_penalty"`
	RawStress       float64 `json:"raw_stress"`
}

type ScheduleResponse struct {
	GeneratedAt     string           `json:"generated_at"`
	Options         []ScheduleOption `json:"options"`
	Source          string           `json:"source"`
	KernelReachable bool             `json:"kernel_reachable"`
	Reason          string           `json:"reason,omitempty"`
	ScoreWeights    scoreWeights     `json:"score_weights"`
	Debug           bool             `json:"debug,omitempty"`
	Pareto          bool             `json:"pareto,omitempty"`
}

// scoreWeights blend meeting signals; values are normalized to sum to 1 when parsing query params.
type scoreWeights struct {
	Weekly     float64 `json:"w_weekly"`
	Evening    float64 `json:"w_evening"`
	Early      float64 `json:"w_early"`
	BackToBack float64 `json:"w_back_to_back"`
	BusyDay    float64 `json:"w_busy_day"`
}

type kernelConflictResponse struct {
	HasConflict bool `json:"has_conflict"`
}

// meetingBlock is one class session from meetings CSV (per section).
type meetingBlock struct {
	DayCode  string
	StartMin int
	EndMin   int
}

type sectionRow struct {
	SubjectCode string
	CourseCode  string
	Section     string
	Credits     float64
}

type scheduleParams struct {
	K                int
	MaxResults       int
	EarliestStart    int
	MaxPerSubject    int
	MinTotalCredits  float64
	MaxTotalCredits  float64
	SubjectWhitelist map[string]struct{}
	SubjectBlacklist map[string]struct{}
	Debug            bool
	Weights          scoreWeights
	Pareto           bool
	ParetoMode       string
	ParetoEpsilon    float64
}

func loadSectionsFromCSV(path string, maxRows int) ([]sectionRow, error) {
	file, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer file.Close()

	reader := csv.NewReader(file)
	header, err := reader.Read()
	if err != nil {
		return nil, err
	}

	indices := map[string]int{}
	for i, name := range header {
		indices[name] = i
	}
	required := []string{"subject_code", "course_code", "section"}
	for _, key := range required {
		if _, ok := indices[key]; !ok {
			return nil, fmt.Errorf("missing csv column: %s", key)
		}
	}

	seen := make(map[string]struct{})
	initialCap := maxRows
	if initialCap <= 0 {
		initialCap = 256
	}
	rows := make([]sectionRow, 0, initialCap)
	for {
		record, readErr := reader.Read()
		if readErr == io.EOF {
			break
		}
		if readErr != nil {
			return nil, readErr
		}

		subject := record[indices["subject_code"]]
		course := record[indices["course_code"]]
		section := record[indices["section"]]
		credits := 1.0
		if ci, ok := indices["credits"]; ok && ci < len(record) {
			if v := strings.TrimSpace(record[ci]); v != "" {
				if f, err := strconv.ParseFloat(v, 64); err == nil && f > 0 {
					credits = f
				}
			}
		}
		key := course + "-" + section
		if _, ok := seen[key]; ok {
			continue
		}
		seen[key] = struct{}{}
		rows = append(rows, sectionRow{
			SubjectCode: subject,
			CourseCode:  course,
			Section:     section,
			Credits:     credits,
		})
		if maxRows > 0 && len(rows) >= maxRows {
			break
		}
	}

	return rows, nil
}

func buildCandidateOptions(rows []sectionRow, target int) []ScheduleOption {
	options := make([]ScheduleOption, 0, target)
	for i := 0; i < len(rows) && len(options) < target; i++ {
		for j := i + 1; j < len(rows) && len(options) < target; j++ {
			if rows[i].CourseCode == rows[j].CourseCode {
				continue
			}
			options = append(options, ScheduleOption{
				Sections: []string{
					rows[i].CourseCode + "-" + rows[i].Section,
					rows[j].CourseCode + "-" + rows[j].Section,
				},
			})
		}
	}
	return assignScheduleIDs(options)
}

func assignScheduleIDs(options []ScheduleOption) []ScheduleOption {
	for i := range options {
		options[i].ID = fmt.Sprintf("sched-%03d", i+1)
	}
	return options
}

func defaultScoreWeights() scoreWeights {
	return scoreWeights{
		Weekly:     0.35,
		Evening:    0.20,
		Early:      0.15,
		BackToBack: 0.15,
		BusyDay:    0.15,
	}
}

func parseScoreWeights(q url.Values) scoreWeights {
	w := defaultScoreWeights()
	parse := func(key string, dest *float64) {
		if v := q.Get(key); v != "" {
			if f, err := strconv.ParseFloat(v, 64); err == nil && f >= 0 {
				*dest = f
			}
		}
	}
	parse("w_weekly", &w.Weekly)
	parse("w_evening", &w.Evening)
	parse("w_early", &w.Early)
	parse("w_back_to_back", &w.BackToBack)
	parse("w_busy_day", &w.BusyDay)
	sum := w.Weekly + w.Evening + w.Early + w.BackToBack + w.BusyDay
	if sum <= 1e-9 {
		return defaultScoreWeights()
	}
	w.Weekly /= sum
	w.Evening /= sum
	w.Early /= sum
	w.BackToBack /= sum
	w.BusyDay /= sum
	return w
}

// scoreSchedule derives stress and utility from aggregated meeting data using configurable weights.
func scoreSchedule(sections []string, bySection map[string][]meetingBlock, weights scoreWeights, includeBreakdown bool) (stress float64, utility float64, academic float64, lifestyle float64, breakdown *ScoreBreakdown) {
	const (
		minPerWeekCap    = 1500.0 // ~25h scheduled time
		eveningMinCap    = 360.0
		earlyMinCap      = 240.0
		backToBackCap    = 8.0
		busyDayMeetCap   = 6.0
		eveningStartMin  = 17 * 60 // 5:00 PM
		earlyEndMin      = 9 * 60  // before 9:00 AM start counts as early
		backToBackGapMin = 12
	)

	var totalMin float64
	var eveningMin float64
	var earlyMin float64
	missingSections := 0

	for _, sec := range sections {
		blocks, ok := bySection[sec]
		if !ok || len(blocks) == 0 {
			missingSections++
			continue
		}
		for _, b := range blocks {
			if b.EndMin <= b.StartMin {
				continue
			}
			dur := float64(b.EndMin - b.StartMin)
			totalMin += dur
			if b.StartMin >= eveningStartMin {
				eveningMin += dur
			}
			if b.StartMin < earlyEndMin {
				earlyMin += dur
			}
		}
	}

	if missingSections > 0 {
		penalty := 0.08 * float64(missingSections)
		totalMin += penalty * minPerWeekCap
	}

	byDay := map[string][]meetingBlock{}
	for _, sec := range sections {
		for _, b := range bySection[sec] {
			if b.EndMin <= b.StartMin {
				continue
			}
			byDay[b.DayCode] = append(byDay[b.DayCode], b)
		}
	}

	backToBack := 0
	busyDayMax := 0
	for _, dayBlocks := range byDay {
		if len(dayBlocks) > busyDayMax {
			busyDayMax = len(dayBlocks)
		}
		sort.Slice(dayBlocks, func(i, j int) bool {
			return dayBlocks[i].StartMin < dayBlocks[j].StartMin
		})
		for i := 1; i < len(dayBlocks); i++ {
			gap := dayBlocks[i].StartMin - dayBlocks[i-1].EndMin
			if gap >= 0 && gap < backToBackGapMin {
				backToBack++
			}
		}
	}

	nWeekly := math.Min(1.0, totalMin/minPerWeekCap)
	nEvening := math.Min(1.0, eveningMin/eveningMinCap)
	nEarly := math.Min(1.0, earlyMin/earlyMinCap)
	nBack := math.Min(1.0, float64(backToBack)/backToBackCap)
	nBusy := math.Min(1.0, float64(busyDayMax)/busyDayMeetCap)

	academicRaw := 0.65*nWeekly + 0.20*nBack + 0.15*nBusy
	lifestyleRaw := 0.65*nEvening + 0.35*nEarly
	academic = roundFloat(math.Min(1.0, academicRaw), 3)
	lifestyle = roundFloat(math.Min(1.0, lifestyleRaw), 3)

	raw := weights.Weekly*nWeekly + weights.Evening*nEvening + weights.Early*nEarly + weights.BackToBack*nBack + weights.BusyDay*nBusy
	stress = roundFloat(math.Min(1.0, raw), 3)
	utility = roundFloat(math.Max(0.0, 1.0-stress), 3)

	if includeBreakdown {
		breakdown = &ScoreBreakdown{
			TotalMinutes:    int(totalMin + 0.5),
			EveningMinutes:  int(eveningMin + 0.5),
			EarlyMinutes:    int(earlyMin + 0.5),
			BackToBackPairs: backToBack,
			BusyDayMax:      busyDayMax,
			MissingSections: missingSections,
			NormWeekly:      roundFloat(nWeekly, 4),
			NormEvening:     roundFloat(nEvening, 4),
			NormEarly:       roundFloat(nEarly, 4),
			NormBackToBack:  roundFloat(nBack, 4),
			NormBusyDay:     roundFloat(nBusy, 4),
			AcademicLoad:    academic,
			LifestylePenalty: lifestyle,
			RawStress:       roundFloat(raw, 4),
		}
	}
	return stress, utility, academic, lifestyle, breakdown
}

func applyScheduleScores(options []ScheduleOption, bySection map[string][]meetingBlock, weights scoreWeights, debug bool) {
	for i := range options {
		s, u, a, l, bd := scoreSchedule(options[i].Sections, bySection, weights, debug)
		options[i].StressScore = s
		options[i].ExpectedUtility = u
		options[i].AcademicLoadScore = a
		options[i].LifestylePenaltyScore = l
		if debug {
			options[i].ScoreBreakdown = bd
		} else {
			options[i].ScoreBreakdown = nil
		}
	}
}

func parseQueryParams(req *http.Request) scheduleParams {
	q := req.URL.Query()
	k := 4
	maxResults := 10
	earliestStart := 0
	maxPerSubject := 1
	subjectWhitelist := parseCSVSet(q.Get("subject_whitelist"))
	subjectBlacklist := parseCSVSet(q.Get("subject_blacklist"))
	debug := q.Get("debug") == "1" || strings.EqualFold(q.Get("debug"), "true")
	pareto := q.Get("pareto") == "1" || strings.EqualFold(q.Get("pareto"), "true")
	paretoMode := strings.ToLower(strings.TrimSpace(q.Get("pareto_mode")))
	if paretoMode == "" {
		paretoMode = "strict"
	}
	if paretoMode != "strict" && paretoMode != "epsilon" {
		paretoMode = "strict"
	}
	paretoEpsilon := 0.03
	if raw := q.Get("pareto_epsilon"); raw != "" {
		if val, err := strconv.ParseFloat(raw, 64); err == nil && val >= 0 && val <= 0.25 {
			paretoEpsilon = val
		}
	}
	weights := parseScoreWeights(q)

	if raw := q.Get("k"); raw != "" {
		if val, err := strconv.Atoi(raw); err == nil && val >= 2 && val <= 6 {
			k = val
		}
	}
	if raw := q.Get("max_results"); raw != "" {
		if val, err := strconv.Atoi(raw); err == nil && val >= 1 && val <= 25 {
			maxResults = val
		}
	}
	if raw := q.Get("earliest_start"); raw != "" {
		if minutes, err := parseAMPM(raw); err == nil {
			earliestStart = minutes
		}
	}
	if raw := q.Get("max_per_subject"); raw != "" {
		if val, err := strconv.Atoi(raw); err == nil && val >= 1 && val <= 3 {
			maxPerSubject = val
		}
	}

	minTotalCredits := 0.0
	maxTotalCredits := 0.0
	if raw := q.Get("min_total_credits"); raw != "" {
		if f, err := strconv.ParseFloat(raw, 64); err == nil && f >= 0 && f <= 40 {
			minTotalCredits = f
		}
	}
	if raw := q.Get("max_total_credits"); raw != "" {
		if f, err := strconv.ParseFloat(raw, 64); err == nil && f >= 0 && f <= 40 {
			maxTotalCredits = f
		}
	}
	if minTotalCredits > 0 && maxTotalCredits > 0 && maxTotalCredits < minTotalCredits {
		maxTotalCredits = minTotalCredits
	}

	return scheduleParams{
		K:                k,
		MaxResults:       maxResults,
		EarliestStart:    earliestStart,
		MaxPerSubject:    maxPerSubject,
		MinTotalCredits:  minTotalCredits,
		MaxTotalCredits:  maxTotalCredits,
		SubjectWhitelist: subjectWhitelist,
		SubjectBlacklist: subjectBlacklist,
		Debug:            debug,
		Weights:          weights,
		Pareto:           pareto,
		ParetoMode:       paretoMode,
		ParetoEpsilon:    paretoEpsilon,
	}
}

func dominates(a, b ScheduleOption) bool {
	betterOrEqualAcademic := a.AcademicLoadScore <= b.AcademicLoadScore
	betterOrEqualLifestyle := a.LifestylePenaltyScore <= b.LifestylePenaltyScore
	strictBetter := a.AcademicLoadScore < b.AcademicLoadScore || a.LifestylePenaltyScore < b.LifestylePenaltyScore
	return betterOrEqualAcademic && betterOrEqualLifestyle && strictBetter
}

func paretoFrontier(options []ScheduleOption) []ScheduleOption {
	frontier := make([]ScheduleOption, 0, len(options))
	for i, candidate := range options {
		dominated := false
		for j, other := range options {
			if i == j {
				continue
			}
			if dominates(other, candidate) {
				dominated = true
				break
			}
		}
		if !dominated {
			frontier = append(frontier, candidate)
		}
	}
	return frontier
}

func epsilonFrontier(options []ScheduleOption, eps float64) []ScheduleOption {
	if len(options) == 0 {
		return nil
	}
	bestAcademic := options[0].AcademicLoadScore
	bestLifestyle := options[0].LifestylePenaltyScore
	for _, opt := range options {
		if opt.AcademicLoadScore < bestAcademic {
			bestAcademic = opt.AcademicLoadScore
		}
		if opt.LifestylePenaltyScore < bestLifestyle {
			bestLifestyle = opt.LifestylePenaltyScore
		}
	}

	out := make([]ScheduleOption, 0, len(options))
	for _, opt := range options {
		if opt.AcademicLoadScore <= bestAcademic+eps && opt.LifestylePenaltyScore <= bestLifestyle+eps {
			out = append(out, opt)
		}
	}
	return out
}

func parseCSVSet(raw string) map[string]struct{} {
	set := map[string]struct{}{}
	for _, item := range strings.Split(raw, ",") {
		value := strings.ToUpper(strings.TrimSpace(item))
		if value == "" {
			continue
		}
		set[value] = struct{}{}
	}
	return set
}

func parseAMPM(value string) (int, error) {
	v := strings.ToUpper(strings.TrimSpace(value))
	if len(v) < 4 {
		return 0, fmt.Errorf("invalid time")
	}
	suffix := v[len(v)-2:]
	if suffix != "AM" && suffix != "PM" {
		return 0, fmt.Errorf("invalid suffix")
	}
	hm := strings.TrimSpace(v[:len(v)-2])
	parts := strings.Split(hm, ":")
	if len(parts) != 2 {
		return 0, fmt.Errorf("invalid hh:mm")
	}
	hour, err := strconv.Atoi(parts[0])
	if err != nil {
		return 0, err
	}
	minute, err := strconv.Atoi(parts[1])
	if err != nil {
		return 0, err
	}
	if hour < 1 || hour > 12 || minute < 0 || minute > 59 {
		return 0, fmt.Errorf("time out of range")
	}
	hour = hour % 12
	if suffix == "PM" {
		hour += 12
	}
	return hour*60 + minute, nil
}

func roundFloat(v float64, places int) float64 {
	p := math.Pow(10, float64(places))
	return math.Round(v*p) / p
}

// loadMeetingsData reads meetings CSV once: earliest start per section (for filters) and all blocks for scoring.
func loadMeetingsData(path string) (map[string]int, map[string][]meetingBlock, error) {
	file, err := os.Open(path)
	if err != nil {
		return nil, nil, err
	}
	defer file.Close()

	reader := csv.NewReader(file)
	header, err := reader.Read()
	if err != nil {
		return nil, nil, err
	}
	indices := map[string]int{}
	for i, name := range header {
		indices[name] = i
	}
	required := []string{"course_code", "section", "day_code", "start_time", "end_time"}
	for _, key := range required {
		if _, ok := indices[key]; !ok {
			return nil, nil, fmt.Errorf("missing csv column: %s", key)
		}
	}

	startTimes := map[string]int{}
	bySection := map[string][]meetingBlock{}

	for {
		record, readErr := reader.Read()
		if readErr == io.EOF {
			break
		}
		if readErr != nil {
			return nil, nil, readErr
		}
		key := record[indices["course_code"]] + "-" + record[indices["section"]]
		startMin, errS := parseAMPM(record[indices["start_time"]])
		if errS != nil {
			continue
		}
		endMin, errE := parseAMPM(record[indices["end_time"]])
		if errE != nil {
			continue
		}
		dayCode := strings.TrimSpace(record[indices["day_code"]])
		if dayCode == "" {
			continue
		}

		if existing, ok := startTimes[key]; !ok || startMin < existing {
			startTimes[key] = startMin
		}
		bySection[key] = append(bySection[key], meetingBlock{
			DayCode:  dayCode,
			StartMin: startMin,
			EndMin:   endMin,
		})
	}
	return startTimes, bySection, nil
}

func sectionPassesEarliestStart(sectionID string, earliestStart int, starts map[string]int) bool {
	if earliestStart == 0 {
		return true
	}
	start, ok := starts[sectionID]
	if !ok {
		return false
	}
	return start >= earliestStart
}

// intervalsOverlap matches kernel pruning: two meetings clash iff each starts before the other ends.
func intervalsOverlap(s1, e1, s2, e2 int) bool {
	return s1 < e2 && s2 < e1
}

func rotateRows(rows []sectionRow, offset int) []sectionRow {
	if len(rows) == 0 {
		return rows
	}
	offset = offset % len(rows)
	if offset == 0 {
		return rows
	}
	rotated := make([]sectionRow, 0, len(rows))
	rotated = append(rotated, rows[offset:]...)
	rotated = append(rotated, rows[:offset]...)
	return rotated
}

func uniqueKeyFromSections(sections []string) string {
	cp := make([]string, len(sections))
	copy(cp, sections)
	sort.Strings(cp)
	return strings.Join(cp, "|")
}

func stackTotalCredits(stack []sectionRow) float64 {
	var t float64
	for _, r := range stack {
		t += r.Credits
	}
	return t
}

func passesCreditBounds(total, minC, maxC float64) bool {
	const eps = 1e-6
	if minC > eps && total+eps < minC {
		return false
	}
	if maxC > eps && total > maxC+eps {
		return false
	}
	return true
}

// companionCoursesCSVPath returns courses_<term>.csv beside sections_<term>.csv, or "" if the name pattern does not match.
func companionCoursesCSVPath(sectionsPath string) string {
	base := filepath.Base(sectionsPath)
	if !strings.HasPrefix(base, "sections_") || !strings.HasSuffix(base, ".csv") {
		return ""
	}
	term := strings.TrimSuffix(strings.TrimPrefix(base, "sections_"), ".csv")
	return filepath.Join(filepath.Dir(sectionsPath), fmt.Sprintf("courses_%s.csv", term))
}

// loadPrereqGroups reads JSON prerequisite clauses from courses CSV (prereq_groups column). Missing file or column yields an empty map.
func loadPrereqGroups(path string) map[string][][]string {
	out := make(map[string][][]string)
	if path == "" {
		return out
	}
	st, err := os.Stat(path)
	if err != nil || st.IsDir() {
		return out
	}
	f, err := os.Open(path)
	if err != nil {
		return out
	}
	defer f.Close()
	reader := csv.NewReader(f)
	header, err := reader.Read()
	if err != nil {
		return out
	}
	indices := map[string]int{}
	for i, name := range header {
		indices[strings.TrimSpace(name)] = i
	}
	ccIdx, ok1 := indices["course_code"]
	pgIdx, ok2 := indices["prereq_groups"]
	if !ok1 || !ok2 {
		return out
	}
	for {
		record, readErr := reader.Read()
		if readErr == io.EOF {
			break
		}
		if readErr != nil {
			break
		}
		if ccIdx >= len(record) || pgIdx >= len(record) {
			continue
		}
		code := strings.ToUpper(strings.TrimSpace(record[ccIdx]))
		if code == "" {
			continue
		}
		raw := strings.TrimSpace(record[pgIdx])
		if raw == "" {
			raw = "[]"
		}
		var groups [][]string
		if err := json.Unmarshal([]byte(raw), &groups); err != nil {
			continue
		}
		norm := make([][]string, 0, len(groups))
		for _, g := range groups {
			ng := make([]string, 0, len(g))
			for _, c := range g {
				u := strings.ToUpper(strings.TrimSpace(c))
				if u != "" {
					ng = append(ng, u)
				}
			}
			if len(ng) > 0 {
				norm = append(norm, ng)
			}
		}
		out[code] = norm
	}
	return out
}

func passesPrereqs(stack []sectionRow, prereqGroups map[string][][]string) bool {
	if len(prereqGroups) == 0 {
		return true
	}
	selected := make(map[string]struct{}, len(stack))
	for _, r := range stack {
		selected[strings.ToUpper(strings.TrimSpace(r.CourseCode))] = struct{}{}
	}
	memo := make(map[string]bool)
	for c := range selected {
		if !coursePrereqsTransitiveSatisfied(c, selected, prereqGroups, memo) {
			return false
		}
	}
	return true
}

// coursePrereqsTransitiveSatisfied is true iff every AND-clause (OR-group) for course is satisfied
// by some alternative in the schedule that is itself recursively satisfied (transitive prereqs).
func coursePrereqsTransitiveSatisfied(course string, selected map[string]struct{}, prereqGroups map[string][][]string, memo map[string]bool) bool {
	return coursePrereqsTransitiveVisit(course, selected, prereqGroups, memo, make(map[string]struct{}))
}

func coursePrereqsTransitiveVisit(course string, selected map[string]struct{}, prereqGroups map[string][][]string, memo map[string]bool, visiting map[string]struct{}) bool {
	if v, ok := memo[course]; ok {
		return v
	}
	if _, cycle := visiting[course]; cycle {
		return false
	}
	clauses, has := prereqGroups[course]
	if !has || len(clauses) == 0 {
		memo[course] = true
		return true
	}
	visiting[course] = struct{}{}
	for _, orGroup := range clauses {
		if len(orGroup) == 0 {
			continue
		}
		groupOK := false
		for _, alt := range orGroup {
			if _, in := selected[alt]; in && coursePrereqsTransitiveVisit(alt, selected, prereqGroups, memo, visiting) {
				groupOK = true
				break
			}
		}
		if !groupOK {
			delete(visiting, course)
			memo[course] = false
			return false
		}
	}
	delete(visiting, course)
	memo[course] = true
	return true
}

func buildCombinationalCandidates(rows []sectionRow, k, maxCandidates int, earliestStart int, maxPerSubject int, starts map[string]int, bySection map[string][]meetingBlock, minTotalCredits, maxTotalCredits float64, prereqGroups map[string][][]string) []ScheduleOption {
	if len(rows) == 0 {
		return nil
	}
	results := make([]ScheduleOption, 0, maxCandidates)
	seen := make(map[string]struct{}, maxCandidates)
	seeds := []int{0, len(rows) / 7, len(rows) / 5, len(rows) / 3, len(rows) / 2}

	for _, seed := range seeds {
		pool := rotateRows(rows, seed)
		stack := make([]sectionRow, 0, k)
		occ := newWeeklyTimeBitmap()

		var dfs func(start int)
		dfs = func(start int) {
			if len(results) >= maxCandidates {
				return
			}
			if len(stack) == k {
				sections := make([]string, 0, k)
				for _, row := range stack {
					sections = append(sections, row.CourseCode+"-"+row.Section)
				}
				if !passesCreditBounds(stackTotalCredits(stack), minTotalCredits, maxTotalCredits) {
					return
				}
				if !passesPrereqs(stack, prereqGroups) {
					return
				}
				key := uniqueKeyFromSections(sections)
				if _, ok := seen[key]; ok {
					return
				}
				seen[key] = struct{}{}
				results = append(results, ScheduleOption{Sections: sections})
				return
			}

			usedCourse := map[string]struct{}{}
			subjectCounts := map[string]int{}
			for _, row := range stack {
				usedCourse[row.CourseCode] = struct{}{}
				subjectCounts[row.SubjectCode]++
			}

			for i := start; i < len(pool); i++ {
				row := pool[i]
				if _, exists := usedCourse[row.CourseCode]; exists {
					continue
				}
				if subjectCounts[row.SubjectCode] >= maxPerSubject {
					continue
				}
				sectionID := row.CourseCode + "-" + row.Section
				if !sectionPassesEarliestStart(sectionID, earliestStart, starts) {
					continue
				}
				newBlocks := bySection[sectionID]
				if !occ.tryAddBlocks(newBlocks) {
					continue
				}
				stack = append(stack, row)
				dfs(i + 1)
				stack = stack[:len(stack)-1]
				occ.removeBlocks(newBlocks)
				if len(results) >= maxCandidates {
					return
				}
			}
		}

		dfs(0)
		if len(results) >= maxCandidates {
			break
		}
	}
	return assignScheduleIDs(results)
}

func filterRowsBySubjects(rows []sectionRow, whitelist, blacklist map[string]struct{}) []sectionRow {
	filtered := make([]sectionRow, 0, len(rows))
	for _, row := range rows {
		if len(whitelist) > 0 {
			if _, ok := whitelist[row.SubjectCode]; !ok {
				continue
			}
		}
		if _, blocked := blacklist[row.SubjectCode]; blocked {
			continue
		}
		filtered = append(filtered, row)
	}
	return filtered
}

func rebalanceRowsBySubject(rows []sectionRow, maxRows int) []sectionRow {
	bySubject := map[string][]sectionRow{}
	subjects := make([]string, 0)
	for _, row := range rows {
		if _, ok := bySubject[row.SubjectCode]; !ok {
			subjects = append(subjects, row.SubjectCode)
		}
		bySubject[row.SubjectCode] = append(bySubject[row.SubjectCode], row)
	}
	sort.Strings(subjects)
	rebalanced := make([]sectionRow, 0, min(maxRows, len(rows)))
	idx := 0
	for len(rebalanced) < maxRows {
		progressed := false
		for _, subject := range subjects {
			entries := bySubject[subject]
			if idx < len(entries) {
				rebalanced = append(rebalanced, entries[idx])
				progressed = true
				if len(rebalanced) == maxRows {
					break
				}
			}
		}
		if !progressed {
			break
		}
		idx++
	}
	return rebalanced
}

func checkConflictsGRPC(ctx context.Context, client optimav1.KernelClient, meetingsCSV string, sections []string) (bool, error) {
	resp, err := client.CheckConflicts(ctx, &optimav1.CheckConflictsRequest{
		CsvPath:  meetingsCSV,
		Sections: sections,
	})
	if err != nil {
		return false, err
	}
	return resp.HasConflict, nil
}

func checkConflictsHTTP(client *http.Client, kernelBaseURL string, sections []string) (bool, error) {
	conflictURL := fmt.Sprintf(
		"%s/v1/conflicts?sections=%s",
		strings.TrimRight(kernelBaseURL, "/"),
		strings.Join(sections, ","),
	)

	req, err := http.NewRequest(http.MethodGet, conflictURL, nil)
	if err != nil {
		return false, err
	}

	resp, err := client.Do(req)
	if err != nil {
		return false, err
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return false, fmt.Errorf("kernel status %d", resp.StatusCode)
	}

	var payload kernelConflictResponse
	if err := json.NewDecoder(resp.Body).Decode(&payload); err != nil {
		return false, err
	}
	return payload.HasConflict, nil
}

func checkConflictsPreferGRPC(
	ctx context.Context,
	httpClient *http.Client,
	grpcClient optimav1.KernelClient,
	kernelHTTP string,
	meetingsCSV string,
	sections []string,
) (bool, error) {
	if grpcClient != nil {
		subCtx, cancel := context.WithTimeout(ctx, 2*time.Second)
		ok, err := checkConflictsGRPC(subCtx, grpcClient, meetingsCSV, sections)
		cancel()
		if err == nil {
			return ok, nil
		}
	}
	return checkConflictsHTTP(httpClient, kernelHTTP, sections)
}

func subjectMapToSlice(m map[string]struct{}) []string {
	if len(m) == 0 {
		return nil
	}
	s := make([]string, 0, len(m))
	for k := range m {
		s = append(s, k)
	}
	sort.Strings(s)
	return s
}

func mapProtoScheduleOptions(opts []*optimav1.ScheduleOption) []ScheduleOption {
	out := make([]ScheduleOption, 0, len(opts))
	for _, o := range opts {
		if o == nil {
			continue
		}
		out = append(out, ScheduleOption{
			ID:                    o.GetId(),
			Sections:              o.GetSections(),
			ExpectedUtility:       o.GetExpectedUtility(),
			StressScore:           o.GetStressScore(),
			AcademicLoadScore:     o.GetAcademicLoadScore(),
			LifestylePenaltyScore: o.GetLifestylePenaltyScore(),
		})
	}
	return out
}

func scoreWeightsFromProto(w *optimav1.ScoreWeights) scoreWeights {
	if w == nil {
		return defaultScoreWeights()
	}
	return scoreWeights{
		Weekly:     w.WWeekly,
		Evening:    w.WEvening,
		Early:      w.WEarly,
		BackToBack: w.WBackToBack,
		BusyDay:    w.WBusyDay,
	}
}

func buildOptimizeRequest(absSections, absMeetings string, params scheduleParams) *optimav1.OptimizeRequest {
	return &optimav1.OptimizeRequest{
		SectionsCsvPath:      absSections,
		MeetingsCsvPath:      absMeetings,
		K:                    int32(params.K),
		MaxResults:           int32(params.MaxResults),
		MaxPerSubject:        int32(params.MaxPerSubject),
		EarliestStartMinutes: int32(params.EarliestStart),
		SubjectWhitelist:     subjectMapToSlice(params.SubjectWhitelist),
		SubjectBlacklist:     subjectMapToSlice(params.SubjectBlacklist),
		Weights: &optimav1.ScoreWeights{
			WWeekly:     params.Weights.Weekly,
			WEvening:    params.Weights.Evening,
			WEarly:      params.Weights.Early,
			WBackToBack: params.Weights.BackToBack,
			WBusyDay:    params.Weights.BusyDay,
		},
		Pareto:            params.Pareto,
		ParetoMode:        params.ParetoMode,
		ParetoEpsilon:     params.ParetoEpsilon,
		MaxCandidates:     2000,
		MinTotalCredits:   params.MinTotalCredits,
		MaxTotalCredits:   params.MaxTotalCredits,
	}
}

func schedulesHandler(w http.ResponseWriter, r *http.Request) {
	client := &http.Client{Timeout: 3 * time.Second}
	kernelBaseURL := "http://localhost:8090"
	grpcAddr := os.Getenv("KERNEL_GRPC_ADDR")
	if grpcAddr == "" {
		grpcAddr = "localhost:50051"
	}
	var grpcClient optimav1.KernelClient
	if conn, err := grpc.NewClient(grpcAddr, grpc.WithTransportCredentials(insecure.NewCredentials())); err == nil {
		defer func() { _ = conn.Close() }()
		grpcClient = optimav1.NewKernelClient(conn)
	}
	params := parseQueryParams(r)
	sectionsPath := filepath.Clean("../python-ml/output/sections_1269.csv")
	meetingsPath := filepath.Clean("../python-ml/output/meetings_1269.csv")
	absSections, errAbsS := filepath.Abs(sectionsPath)
	if errAbsS != nil {
		absSections = sectionsPath
	}
	absMeetings, errAbsM := filepath.Abs(meetingsPath)
	if errAbsM != nil {
		absMeetings = meetingsPath
	}

	useKernelOptimize := grpcClient != nil &&
		os.Getenv("ORCHESTRATOR_USE_KERNEL_OPTIMIZE") != "0" &&
		r.URL.Query().Get("legacy") != "1"

	if useKernelOptimize {
		req := buildOptimizeRequest(absSections, absMeetings, params)
		optCtx, cancel := context.WithTimeout(r.Context(), 60*time.Second)
		defer cancel()
		out, err := grpcClient.Optimize(optCtx, req)
		if err == nil && out.GetStatus() == "ok" {
			weights := params.Weights
			if ew := out.GetEffectiveWeights(); ew != nil {
				weights = scoreWeightsFromProto(ew)
			}
			reason := out.GetReason()
			resp := ScheduleResponse{
				GeneratedAt:     time.Now().UTC().Format(time.RFC3339),
				Source:            "orchestrator+kernel",
				Options:           mapProtoScheduleOptions(out.GetOptions()),
				KernelReachable:   true,
				Reason:            reason,
				ScoreWeights:      weights,
				Debug:             params.Debug,
				Pareto:            params.Pareto,
			}
			w.Header().Set("Content-Type", "application/json")
			_ = json.NewEncoder(w).Encode(resp)
			return
		}
	}

	rows, err := loadSectionsFromCSV(sectionsPath, 0)
	if err != nil {
		http.Error(w, "failed to load sections csv: "+err.Error(), http.StatusInternalServerError)
		return
	}
	rows = filterRowsBySubjects(rows, params.SubjectWhitelist, params.SubjectBlacklist)
	if len(rows) == 0 {
		resp := ScheduleResponse{
			GeneratedAt:     time.Now().UTC().Format(time.RFC3339),
			Source:          "orchestrator",
			KernelReachable: false,
			Reason:          "no_sections_after_subject_filters",
			ScoreWeights:    params.Weights,
			Debug:           params.Debug,
		}
		w.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(w).Encode(resp)
		return
	}
	rows = rebalanceRowsBySubject(rows, 180)
	startTimes, meetingsBySection, err := loadMeetingsData(meetingsPath)
	if err != nil {
		http.Error(w, "failed to load meetings csv: "+err.Error(), http.StatusInternalServerError)
		return
	}

	prereqMap := loadPrereqGroups(companionCoursesCSVPath(sectionsPath))
	candidateOptions := buildCombinationalCandidates(rows, params.K, 2000, params.EarliestStart, params.MaxPerSubject, startTimes, meetingsBySection, params.MinTotalCredits, params.MaxTotalCredits, prereqMap)
	if len(candidateOptions) == 0 {
		resp := ScheduleResponse{
			GeneratedAt:     time.Now().UTC().Format(time.RFC3339),
			Source:          "orchestrator",
			KernelReachable: false,
			Reason:          "no_candidates_generated",
			ScoreWeights:    params.Weights,
			Debug:           params.Debug,
		}
		w.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(w).Encode(resp)
		return
	}
	applyScheduleScores(candidateOptions, meetingsBySection, params.Weights, params.Debug)
	sort.SliceStable(candidateOptions, func(i, j int) bool {
		if candidateOptions[i].ExpectedUtility != candidateOptions[j].ExpectedUtility {
			return candidateOptions[i].ExpectedUtility > candidateOptions[j].ExpectedUtility
		}
		return candidateOptions[i].StressScore < candidateOptions[j].StressScore
	})
	filtered := make([]ScheduleOption, 0, len(candidateOptions))
	kernelReachable := true

	for _, option := range candidateOptions {
		hasConflict, checkErr := checkConflictsPreferGRPC(r.Context(), client, grpcClient, kernelBaseURL, meetingsPath, option.Sections)
		if checkErr != nil {
			// Degraded mode: return static options if kernel is unavailable.
			kernelReachable = false
			filtered = candidateOptions[:min(params.MaxResults, len(candidateOptions))]
			break
		}
		if !hasConflict {
			filtered = append(filtered, option)
			if !params.Pareto && len(filtered) == params.MaxResults {
				break
			}
		}
	}
	if params.Pareto && kernelReachable {
		conflictSafe := append([]ScheduleOption(nil), filtered...)
		if params.ParetoMode == "epsilon" {
			filtered = epsilonFrontier(conflictSafe, params.ParetoEpsilon)
		} else {
			filtered = paretoFrontier(conflictSafe)
			// Auto-expand if strict frontier is too narrow.
			if len(filtered) < min(3, params.MaxResults) {
				filtered = epsilonFrontier(conflictSafe, params.ParetoEpsilon)
			}
		}
		sort.SliceStable(filtered, func(i, j int) bool {
			if filtered[i].ExpectedUtility != filtered[j].ExpectedUtility {
				return filtered[i].ExpectedUtility > filtered[j].ExpectedUtility
			}
			return filtered[i].StressScore < filtered[j].StressScore
		})
		if len(filtered) > params.MaxResults {
			filtered = filtered[:params.MaxResults]
		}
	}

	source := "orchestrator+kernel"
	if !kernelReachable {
		source = "orchestrator-fallback"
	}

	resp := ScheduleResponse{
		GeneratedAt:     time.Now().UTC().Format(time.RFC3339),
		Source:          source,
		Options:         filtered,
		KernelReachable: kernelReachable,
		ScoreWeights:    params.Weights,
		Debug:           params.Debug,
		Pareto:          params.Pareto,
	}
	if kernelReachable && len(filtered) == 0 {
		resp.Reason = "no_conflict_free_options_found"
	}

	w.Header().Set("Content-Type", "application/json")
	if err := json.NewEncoder(w).Encode(resp); err != nil {
		http.Error(w, "encode error", http.StatusInternalServerError)
	}
}

func min(a, b int) int {
	if a < b {
		return a
	}
	return b
}

func healthHandler(w http.ResponseWriter, _ *http.Request) {
	w.WriteHeader(http.StatusOK)
	_, _ = w.Write([]byte("ok"))
}

func main() {
	ctx := context.Background()
	shutdownOTel, err := setupOTel(ctx)
	if err != nil {
		log.Printf("otel setup: %v (continuing)", err)
	} else if shutdownOTel != nil {
		defer func() {
			if err := shutdownOTel(context.Background()); err != nil {
				log.Printf("otel shutdown: %v", err)
			}
		}()
	}

	mux := http.NewServeMux()
	mux.HandleFunc("/health", healthHandler)
	mux.HandleFunc("/v1/schedules", schedulesHandler)
	mux.HandleFunc("/v1/agent/plan", agentPlanHandler)

	addr := ":8080"
	handler := instrumentHTTP(mux)
	log.Printf("go-orchestrator listening on %s", addr)
	if err := http.ListenAndServe(addr, handler); err != nil {
		log.Fatal(err)
	}
}
