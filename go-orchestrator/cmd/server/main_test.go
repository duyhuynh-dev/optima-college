package main

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"runtime"
	"testing"
)

func TestHealthHandler(t *testing.T) {
	t.Parallel()
	rec := httptest.NewRecorder()
	req := httptest.NewRequest(http.MethodGet, "/health", nil)
	healthHandler(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("status = %d, want %d", rec.Code, http.StatusOK)
	}
	if got := rec.Body.String(); got != "ok" {
		t.Fatalf("body = %q, want ok", got)
	}
}

// chdirGoOrchestrator sets working directory to go-orchestrator/ (parent of cmd/server)
// so paths like ../python-ml/output/*.csv match `go run ./cmd/server` from that directory.
func chdirGoOrchestrator(t *testing.T) (restore func()) {
	t.Helper()
	_, file, _, ok := runtime.Caller(1)
	if !ok {
		t.Fatal("runtime.Caller failed")
	}
	goOrch := filepath.Clean(filepath.Join(filepath.Dir(file), ".."))
	prev, err := os.Getwd()
	if err != nil {
		t.Fatalf("getwd: %v", err)
	}
	if err := os.Chdir(goOrch); err != nil {
		t.Fatalf("chdir %s: %v", goOrch, err)
	}
	return func() {
		_ = os.Chdir(prev)
	}
}

func TestIntervalsOverlap(t *testing.T) {
	t.Parallel()
	if !intervalsOverlap(530, 610, 570, 620) {
		t.Fatal("expected overlapping Tuesday blocks to clash")
	}
	if intervalsOverlap(480, 540, 540, 600) {
		t.Fatal("back-to-back should not overlap")
	}
}

func TestWeeklyTimeBitmap_overlapSemantics(t *testing.T) {
	t.Parallel()
	occ := newWeeklyTimeBitmap()
	a := []meetingBlock{{DayCode: "T", StartMin: 530, EndMin: 610}}
	if !occ.tryAddBlocks(a) {
		t.Fatal("first add should succeed")
	}
	if occ.tryAddBlocks([]meetingBlock{{DayCode: "T", StartMin: 570, EndMin: 620}}) {
		t.Fatal("expected overlap on same day")
	}
	if !occ.tryAddBlocks([]meetingBlock{{DayCode: "T", StartMin: 620, EndMin: 660}}) {
		t.Fatal("non-overlapping same day should pass")
	}
	occ2 := newWeeklyTimeBitmap()
	if !occ2.tryAddBlocks(a) {
		t.Fatal("setup")
	}
	if !occ2.tryAddBlocks([]meetingBlock{{DayCode: "R", StartMin: 570, EndMin: 620}}) {
		t.Fatal("different day should pass")
	}
}

func TestBuildCombinationalCandidates_PruneTimeOverlap(t *testing.T) {
	t.Parallel()
	rows := []sectionRow{
		{SubjectCode: "COMP", CourseCode: "COMP100", Section: "01", Credits: 1},
		{SubjectCode: "COMP", CourseCode: "COMP101", Section: "01", Credits: 1},
		{SubjectCode: "MATH", CourseCode: "MATH121", Section: "01", Credits: 1},
	}
	by := map[string][]meetingBlock{
		"COMP100-01": {{DayCode: "T", StartMin: 530, EndMin: 610}},
		"COMP101-01": {{DayCode: "T", StartMin: 570, EndMin: 620}},
		"MATH121-01": {{DayCode: "W", StartMin: 600, EndMin: 660}},
	}
	starts := map[string]int{
		"COMP100-01": 530,
		"COMP101-01": 570,
		"MATH121-01": 600,
	}
	opts := buildCombinationalCandidates(rows, 2, 50, 0, 2, starts, by, 0, 0, nil)
	var has100_121, has101_121, has100_101 bool
	for _, o := range opts {
		set := map[string]struct{}{}
		for _, s := range o.Sections {
			set[s] = struct{}{}
		}
		if _, a := set["COMP100-01"]; a {
			if _, b := set["MATH121-01"]; b {
				has100_121 = true
			}
			if _, b := set["COMP101-01"]; b {
				has100_101 = true
			}
		}
		if _, a := set["COMP101-01"]; a {
			if _, b := set["MATH121-01"]; b {
				has101_121 = true
			}
		}
	}
	if !has100_121 || !has101_121 {
		t.Fatalf("expected non-overlapping pairs with MATH121, got opts=%d", len(opts))
	}
	if has100_101 {
		t.Fatal("COMP100 + COMP101 overlap on Tuesday and should be pruned")
	}
}

func TestPassesPrereqs_TransitiveChain(t *testing.T) {
	t.Parallel()
	pg := map[string][][]string{
		"COMP300": {{"COMP200"}},
		"COMP200": {{"COMP112"}},
	}
	full := []sectionRow{{CourseCode: "COMP300"}, {CourseCode: "COMP200"}, {CourseCode: "COMP112"}}
	if !passesPrereqs(full, pg) {
		t.Fatal("expected full chain ok")
	}
	skip := []sectionRow{{CourseCode: "COMP300"}, {CourseCode: "COMP112"}}
	if passesPrereqs(skip, pg) {
		t.Fatal("expected missing middle course to fail")
	}
}

func TestPassesPrereqs_OrAlternativeNeedsOwnPrereqs(t *testing.T) {
	t.Parallel()
	pg := map[string][][]string{
		"ECON110": {{"MATH120", "MATH121"}},
		"MATH121": {{"MATH120"}},
	}
	bad := []sectionRow{{CourseCode: "ECON110"}, {CourseCode: "MATH121"}}
	if passesPrereqs(bad, pg) {
		t.Fatal("MATH121 without MATH120 in schedule should fail")
	}
	ok := []sectionRow{{CourseCode: "ECON110"}, {CourseCode: "MATH120"}, {CourseCode: "MATH121"}}
	if !passesPrereqs(ok, pg) {
		t.Fatal("expected ECON110 + MATH120 + MATH121 ok")
	}
}

func TestPassesPrereqs_Cycle(t *testing.T) {
	t.Parallel()
	pg := map[string][][]string{
		"A": {{"B"}},
		"B": {{"A"}},
	}
	stack := []sectionRow{{CourseCode: "A"}, {CourseCode: "B"}}
	if passesPrereqs(stack, pg) {
		t.Fatal("prereq cycle should fail")
	}
}

func TestSchedulesHandler_Smoke(t *testing.T) {
	restore := chdirGoOrchestrator(t)
	defer restore()

	sectionsPath := filepath.Clean("../python-ml/output/sections_1269.csv")
	if _, err := os.Stat(sectionsPath); err != nil {
		t.Skipf("no ingested data (run `make ingest` from repo root): %v", err)
	}

	req := httptest.NewRequest(http.MethodGet, "/v1/schedules?k=2&max_results=2&legacy=1", nil)
	rec := httptest.NewRecorder()
	schedulesHandler(rec, req)

	if rec.Code != http.StatusOK {
		t.Fatalf("status = %d body = %s", rec.Code, rec.Body.String())
	}

	var resp ScheduleResponse
	if err := json.NewDecoder(rec.Body).Decode(&resp); err != nil {
		t.Fatalf("json: %v", err)
	}
	if resp.GeneratedAt == "" {
		t.Error("empty generated_at")
	}
	if len(resp.Options) == 0 && resp.Reason == "" && resp.Source != "orchestrator" {
		t.Logf("note: empty options (kernel may be down); source=%q reason=%q", resp.Source, resp.Reason)
	}
}
