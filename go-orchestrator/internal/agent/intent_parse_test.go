package agent

import (
	"encoding/json"
	"testing"
)

func TestParseAndValidateClarification(t *testing.T) {
	raw := []byte(`{
		"schema_version": "schedule_intent_v1",
		"clarification_needed": true,
		"clarification_questions": ["How many courses do you want to take?"]
	}`)
	doc, err := ParseAndValidate(raw)
	if err != nil {
		t.Fatal(err)
	}
	if !doc.ClarificationNeeded || len(doc.ClarificationQuestions) != 1 {
		t.Fatalf("%+v", doc)
	}
}

func TestParseAndValidateActionable(t *testing.T) {
	k := 4
	doc := ScheduleIntentV1{
		SchemaVersion:       SchemaVersionScheduleIntentV1,
		ClarificationNeeded: false,
		Intent: &ScheduleIntentFields{
			K: &k,
		},
	}
	raw, _ := json.Marshal(doc)
	out, err := ParseAndValidate(raw)
	if err != nil {
		t.Fatal(err)
	}
	if out.Intent == nil || *out.Intent.K != 4 {
		t.Fatalf("%+v", out)
	}
}
