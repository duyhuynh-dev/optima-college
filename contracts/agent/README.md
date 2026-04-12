# Agent contracts (Phase 4)

## Versioned JSON schema

- **`schedule_intent_v1.schema.json`** — canonical shape the model must return. Bump version (e.g. `schedule_intent_v2`) when breaking fields change; keep old schemas in-repo for replay/tests.
- **Go embed copy:** `go-orchestrator/internal/agent/schedule_intent_v1.schema.json` must stay identical to this file (Go `embed` cannot reference paths outside the module).

## Allowlist (what the agent may trigger)

The agent **does not** call arbitrary HTTP or tools. In production it only:

1. Returns validated JSON matching the schema above.
2. The **orchestrator** maps `intent` → internal **`GET /v1/schedules`** query parameters (`k`, weights, `pareto`, `min_total_credits` / `max_total_credits`, …) and runs the existing optimizer path (kernel gRPC or legacy).

**HTTP surface:** `POST /v1/agent/plan` (orchestrator) is the only agent entrypoint; it allowlists the internal schedule engine as above.

No other routes, no user-supplied URLs, no shell, no database writes from the model output.

## Clarification policy

- **`clarification_needed: true`** when required preferences are missing or ambiguous (e.g. number of courses, subject areas, hard time blocks).
- Emit up to **3** concrete `clarification_questions`.
- **`clarification_needed: false`** when the request is actionable; fill `intent` with explicit values or rely on documented server defaults (see orchestrator).
- Prefer **assumptions_made** + defaults over excessive questioning; only block when optimization would be meaningless.

## Privacy (prompt boundary)

**Never** put in the model prompt:

- Full course catalogs, section lists, meeting rows, or scraped HTML.
- Any PII beyond what the user typed in their message.

**Allowed** in the system/developer prompt:

- Schema text, allowlist description, clarification rules, and **aggregate** hints if enabled server-side (e.g. “~N subjects in term dataset”) — off by default via `ORCHESTRATOR_AGENT_CATALOG_STATS`.

The orchestrator loads CSVs **only** after intent validation, on the server, for `/v1/schedules`.
