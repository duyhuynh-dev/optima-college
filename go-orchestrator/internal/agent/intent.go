package agent

import (
	"encoding/json"
	"fmt"
	"net/url"
	"strings"
)

const SchemaVersionScheduleIntentV1 = "schedule_intent_v1"

// IntentWeights maps to query params w_weekly, w_evening, etc.
type IntentWeights struct {
	Weekly     *float64 `json:"weekly,omitempty"`
	Evening    *float64 `json:"evening,omitempty"`
	Early      *float64 `json:"early,omitempty"`
	BackToBack *float64 `json:"back_to_back,omitempty"`
	BusyDay    *float64 `json:"busy_day,omitempty"`
}

// ScheduleIntentFields are mapped to the internal /v1/schedules API (allowlisted).
type ScheduleIntentFields struct {
	K                *int            `json:"k,omitempty"`
	MaxResults       *int            `json:"max_results,omitempty"`
	EarliestStart    *string         `json:"earliest_start,omitempty"`
	MaxPerSubject    *int            `json:"max_per_subject,omitempty"`
	MinTotalCredits  *float64        `json:"min_total_credits,omitempty"`
	MaxTotalCredits  *float64        `json:"max_total_credits,omitempty"`
	SubjectWhitelist []string        `json:"subject_whitelist,omitempty"`
	SubjectBlacklist []string        `json:"subject_blacklist,omitempty"`
	Debug            *bool           `json:"debug,omitempty"`
	Pareto           *bool           `json:"pareto,omitempty"`
	ParetoMode       *string         `json:"pareto_mode,omitempty"`
	ParetoEpsilon    *float64        `json:"pareto_epsilon,omitempty"`
	Weights          *IntentWeights  `json:"weights,omitempty"`
	Legacy           *bool           `json:"legacy,omitempty"`
}

// ScheduleIntentV1 is the versioned envelope the model must return.
type ScheduleIntentV1 struct {
	SchemaVersion           string                `json:"schema_version"`
	ClarificationNeeded     bool                  `json:"clarification_needed"`
	ClarificationQuestions  []string              `json:"clarification_questions,omitempty"`
	Intent                  *ScheduleIntentFields `json:"intent,omitempty"`
	AssumptionsMade         []string              `json:"assumptions_made,omitempty"`
	Rationale               string                `json:"rationale,omitempty"`
}

// ParseAndValidate unmarshals and checks the envelope (trust boundary after OpenAI).
func ParseAndValidate(raw []byte) (*ScheduleIntentV1, error) {
	var doc ScheduleIntentV1
	if err := json.Unmarshal(raw, &doc); err != nil {
		return nil, fmt.Errorf("intent json: %w", err)
	}
	if doc.SchemaVersion != SchemaVersionScheduleIntentV1 {
		return nil, fmt.Errorf("unsupported schema_version %q (want %s)", doc.SchemaVersion, SchemaVersionScheduleIntentV1)
	}
	if len(doc.Rationale) > 2000 {
		return nil, fmt.Errorf("rationale too long")
	}
	if len(doc.ClarificationQuestions) > 3 {
		return nil, fmt.Errorf("too many clarification_questions")
	}
	if len(doc.AssumptionsMade) > 8 {
		return nil, fmt.Errorf("too many assumptions_made")
	}
	if doc.ClarificationNeeded {
		for _, q := range doc.ClarificationQuestions {
			if strings.TrimSpace(q) == "" {
				return nil, fmt.Errorf("empty clarification question")
			}
			if len(q) > 280 {
				return nil, fmt.Errorf("clarification question too long")
			}
		}
		return &doc, nil
	}
	// Actionable path: validate intent sub-object if present
	if doc.Intent != nil {
		if err := validateIntentFields(doc.Intent); err != nil {
			return nil, err
		}
	}
	return &doc, nil
}

func validateIntentFields(i *ScheduleIntentFields) error {
	if i == nil {
		return nil
	}
	if i.K != nil && (*i.K < 2 || *i.K > 6) {
		return fmt.Errorf("intent.k out of range")
	}
	if i.MaxResults != nil && (*i.MaxResults < 1 || *i.MaxResults > 25) {
		return fmt.Errorf("intent.max_results out of range")
	}
	if i.MaxPerSubject != nil && (*i.MaxPerSubject < 1 || *i.MaxPerSubject > 3) {
		return fmt.Errorf("intent.max_per_subject out of range")
	}
	if i.MinTotalCredits != nil && (*i.MinTotalCredits < 0 || *i.MinTotalCredits > 40) {
		return fmt.Errorf("intent.min_total_credits out of range")
	}
	if i.MaxTotalCredits != nil && (*i.MaxTotalCredits < 0 || *i.MaxTotalCredits > 40) {
		return fmt.Errorf("intent.max_total_credits out of range")
	}
	if i.MinTotalCredits != nil && i.MaxTotalCredits != nil && *i.MinTotalCredits > 0 && *i.MaxTotalCredits > 0 && *i.MaxTotalCredits < *i.MinTotalCredits {
		return fmt.Errorf("intent.max_total_credits below min_total_credits")
	}
	if i.EarliestStart != nil && len(*i.EarliestStart) > 16 {
		return fmt.Errorf("intent.earliest_start too long")
	}
	if len(i.SubjectWhitelist) > 20 || len(i.SubjectBlacklist) > 20 {
		return fmt.Errorf("too many subject filters")
	}
	for _, s := range i.SubjectWhitelist {
		if len(s) < 2 || len(s) > 8 {
			return fmt.Errorf("invalid subject_whitelist entry")
		}
	}
	for _, s := range i.SubjectBlacklist {
		if len(s) < 2 || len(s) > 8 {
			return fmt.Errorf("invalid subject_blacklist entry")
		}
	}
	if i.ParetoMode != nil {
		m := strings.ToLower(strings.TrimSpace(*i.ParetoMode))
		if m != "strict" && m != "epsilon" {
			return fmt.Errorf("invalid pareto_mode")
		}
	}
	if i.ParetoEpsilon != nil && (*i.ParetoEpsilon < 0 || *i.ParetoEpsilon > 0.25) {
		return fmt.Errorf("pareto_epsilon out of range")
	}
	if i.Weights != nil {
		w := i.Weights
		for _, p := range []*float64{w.Weekly, w.Evening, w.Early, w.BackToBack, w.BusyDay} {
			if p != nil && (*p < 0 || *p > 10) {
				return fmt.Errorf("weight out of range")
			}
		}
	}
	return nil
}

// IntentToScheduleQuery builds allowlisted query string for GET /v1/schedules.
func IntentToScheduleQuery(intent *ScheduleIntentFields) string {
	if intent == nil {
		return ""
	}
	var b strings.Builder
	add := func(key, val string) {
		if b.Len() > 0 {
			b.WriteByte('&')
		}
		b.WriteString(key)
		b.WriteByte('=')
		b.WriteString(val)
	}
	if intent.K != nil {
		add("k", fmt.Sprintf("%d", *intent.K))
	}
	if intent.MaxResults != nil {
		add("max_results", fmt.Sprintf("%d", *intent.MaxResults))
	}
	if intent.EarliestStart != nil && strings.TrimSpace(*intent.EarliestStart) != "" {
		add("earliest_start", url.QueryEscape(strings.TrimSpace(*intent.EarliestStart)))
	}
	if intent.MaxPerSubject != nil {
		add("max_per_subject", fmt.Sprintf("%d", *intent.MaxPerSubject))
	}
	if intent.MinTotalCredits != nil && *intent.MinTotalCredits > 0 {
		add("min_total_credits", fmt.Sprintf("%g", *intent.MinTotalCredits))
	}
	if intent.MaxTotalCredits != nil && *intent.MaxTotalCredits > 0 {
		add("max_total_credits", fmt.Sprintf("%g", *intent.MaxTotalCredits))
	}
	if len(intent.SubjectWhitelist) > 0 {
		add("subject_whitelist", url.QueryEscape(strings.Join(intent.SubjectWhitelist, ",")))
	}
	if len(intent.SubjectBlacklist) > 0 {
		add("subject_blacklist", url.QueryEscape(strings.Join(intent.SubjectBlacklist, ",")))
	}
	if intent.Debug != nil && *intent.Debug {
		add("debug", "1")
	}
	if intent.Pareto != nil && *intent.Pareto {
		add("pareto", "1")
	}
	if intent.ParetoMode != nil {
		add("pareto_mode", url.QueryEscape(strings.TrimSpace(*intent.ParetoMode)))
	}
	if intent.ParetoEpsilon != nil {
		add("pareto_epsilon", fmt.Sprintf("%g", *intent.ParetoEpsilon))
	}
	if intent.Weights != nil {
		w := intent.Weights
		if w.Weekly != nil {
			add("w_weekly", fmt.Sprintf("%g", *w.Weekly))
		}
		if w.Evening != nil {
			add("w_evening", fmt.Sprintf("%g", *w.Evening))
		}
		if w.Early != nil {
			add("w_early", fmt.Sprintf("%g", *w.Early))
		}
		if w.BackToBack != nil {
			add("w_back_to_back", fmt.Sprintf("%g", *w.BackToBack))
		}
		if w.BusyDay != nil {
			add("w_busy_day", fmt.Sprintf("%g", *w.BusyDay))
		}
	}
	if intent.Legacy != nil && *intent.Legacy {
		add("legacy", "1")
	}
	return b.String()
}
