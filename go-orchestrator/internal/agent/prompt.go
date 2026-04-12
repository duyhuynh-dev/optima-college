package agent

import (
	"fmt"
	"os"
	"strings"
)

// BuildSystemPrompt includes schema + policy; no catalog rows.
func BuildSystemPrompt() string {
	var b strings.Builder
	b.WriteString(`You are Optima's schedule planning assistant. You MUST respond with a single JSON object only (no markdown).
Rules:
- Output must validate against this JSON Schema (conceptually match field names and types):
`)
	b.WriteString(ScheduleIntentV1Schema)
	b.WriteString(`

Policy:
- Never invent specific course codes, section IDs, or meeting times from a catalog; you only output search preferences.
- The server will run the optimizer after you respond; you only emit structured intent.
- If the user request is too vague to run a meaningful search, set "clarification_needed": true and 1-3 specific "clarification_questions".
- If you can proceed, set "clarification_needed": false and put search knobs under "intent" (omit keys to keep server defaults).
- Always set "schema_version" to "schedule_intent_v1".
- Use "assumptions_made" for defaults you applied (max 8 short strings).
- Use "rationale" for a brief user-facing summary (no PII beyond what the user said).
`)

	if stats := strings.TrimSpace(os.Getenv("ORCHESTRATOR_AGENT_CATALOG_STATS")); stats != "" {
		b.WriteString("\nAggregate catalog hint (not row-level): ")
		b.WriteString(stats)
		b.WriteByte('\n')
	}

	return b.String()
}

// BuildUserMessage wraps the user text (already server-controlled) with locale hint.
func BuildUserMessage(userText, locale string) string {
	loc := strings.TrimSpace(locale)
	if loc == "" {
		loc = "en"
	}
	return fmt.Sprintf("User locale: %s\nUser request:\n%s", loc, strings.TrimSpace(userText))
}
