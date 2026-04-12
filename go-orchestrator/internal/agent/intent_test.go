package agent

import (
	"strings"
	"testing"
)

func TestIntentToScheduleQuery(t *testing.T) {
	k := 4
	mr := 5
	early := "10:00AM"
	pm := "epsilon"
	pe := 0.05
	wEvening := 0.6
	intent := &ScheduleIntentFields{
		K:             &k,
		MaxResults:    &mr,
		EarliestStart: &early,
		ParetoMode:    &pm,
		ParetoEpsilon: &pe,
		Weights:       &IntentWeights{Evening: &wEvening},
	}
	q := IntentToScheduleQuery(intent)
	if q == "" {
		t.Fatal("expected query")
	}
	if !strings.Contains(q, "k=4") || !strings.Contains(q, "max_results=5") || !strings.Contains(q, "earliest_start=") {
		t.Fatalf("unexpected q: %s", q)
	}
	if !strings.Contains(q, "pareto_mode=epsilon") || !strings.Contains(q, "pareto_epsilon=0.05") {
		t.Fatalf("pareto: %s", q)
	}
	if !strings.Contains(q, "w_evening=0.6") {
		t.Fatalf("weights: %s", q)
	}
}
