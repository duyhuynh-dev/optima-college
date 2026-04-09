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
