# Optima — phased plan & progress

> **Document policy (stable):** This file is the agreed **program baseline**. Do not edit it for routine implementation updates, progress ticks, or “sync with main” housekeeping. Revise it only after **explicit stakeholder consultation** when the plan itself changes (phases, checkpoints, or scope). Track execution in issues, PRs, or release notes—not here.

**Why this exists:** A single, detailed roadmap so work stays **ordered from start to finish** (phases → checkpoints). **Operational runbooks** live in [`README.md`](../README.md), [`CONTRIBUTING.md`](../CONTRIBUTING.md), and [`infra/`](../infra/); **scope and sequencing** live here.

**Execution discipline:** Follow the phased plan; **bugs and breakages** are fixed and then work returns to the current phase/checkpoint—no ad hoc rescoping without updating this document under the policy above.

This document is the **structured program plan**. Last reviewed: 2026-04-11.

**Legend:** ✅ Done · 🟡 Partial / in flight · ⬜ Not started

**Critical path (dependency order):** Phase **1** data foundation → **2–3** kernel + orchestration (vertical slice) → **4+** intelligence/product/pilot. Later phases assume earlier checkpoints where noted.

---

## How the current repo maps (honest snapshot)

| Your phase | Match today |
|------------|-------------|
| **0** | 🟡 Repo layout, `Makefile`, CI, protobuf, Jaeger + OTLP. PR template + branching in [`CONTRIBUTING.md`](../CONTRIBUTING.md). Missing: issue tracker choice, Terraform, KPI dashboard skeleton. |
| **1** | 🟡 **Checkpoint A closed (ops).** Ingest → CSV + bronze; **GCS** `make gcs-bronze`; **BQ** + scheduled [`data-pipeline.yml`](../.github/workflows/data-pipeline.yml) + `dq_check`. Full entity DDL / alerting still future. |
| **2** | ✅ **Checkpoint B (MVP).** Credits + **`prereq_groups`** + **transitive** prereq enforcement + **Rayon** + **Pareto** + **bitset-backed** weekly minute maps for DFS time pruning (Rust + Go legacy) + full pairwise **`detect_conflicts`** + **Criterion** bench (`cargo bench --no-run` in CI). *Out of scope for this checkpoint:* prior completed-course credit, co-reqs as first-class constraints, automated perf regression thresholds in CI. |
| **3** | 🟡 gRPC `CheckConflicts` + `Optimize`, Go gateway, degraded **legacy** path. Missing: richer proto (weights/explanations in one shot was partially done), circuit breaker, Redis. |
| **4–8** | ⬜ Not started (NL intent, workload ML, frontend, pilot). |

**Verdict:** Data → Rust kernel → Go API + observability matches the plan. **Phase 1 checkpoint A** met for pipeline + minimal DQ. **Phase 2 checkpoint B** is met for the **scoped MVP** (hard constraints + multi-objective slice); remaining modeling depth (prior credit, richer catalog entities, CI perf gates) is **Phase 3+ polish** or product policy, not blockers for leaving Phase 2.

---

## Phase 0: Program Setup (Week 0)

**Tech:** GitHub, Linear/Jira/Notion, Docker, Makefile, OpenTelemetry collector, Terraform (optional)

| What we do | Progress | Notes |
|------------|----------|--------|
| Mono-repo structure (`go-orchestrator`, `rust-kernel`, data/ML, contracts, infra) | ✅ | `python-ml/` = data pipeline; no separate `data-pipeline/` repo. |
| Engineering standards: branching, PR template, testing gates, release tags | 🟡 | CI: [`.github/workflows/ci.yml`](../.github/workflows/ci.yml). [`.github/pull_request_template.md`](../.github/pull_request_template.md) + branching in [`CONTRIBUTING.md`](../CONTRIBUTING.md). |
| KPI dashboard skeleton (latency, validity, freshness, uptime) | ⬜ | |
| Data contracts + protobuf schemas early | 🟡 | [`contracts/proto/optima/v1/kernel.proto`](../contracts/proto/optima/v1/kernel.proto) — extend as API grows. |

**Expected outcome:** Team can ship predictably with versioned APIs, reproducible local dev, measurable progress.

---

## Phase 1: Data Foundation (Weeks 1–2)

### 1.1 Public catalog ingestion (WesMaps)

**Tech:** Crawler (Python), Cloud Storage, BigQuery

| What we do | Progress | Notes |
|------------|----------|--------|
| Crawl by term / subject | 🟡 | `make ingest` / `optima_ingest` → CSV under `python-ml/output/`. |
| Bronze: raw HTML snapshots | 🟡 | Local bronze HTML optional (`--no-bronze` to skip). **GCS bronze** via `make gcs-bronze` (bucket env); optional in ops. |
| Silver: structured tables | 🟡 | CSV + solver-ready paths; **BigQuery** tables + loader (see `infra/bigquery/`). |

### 1.2 Canonical schema + IDs

**Tech:** BigQuery DDL, dbt (optional), JSON schema / protobuf

| What we do | Progress | Notes |
|------------|----------|--------|
| Entities: catalog, offering, section, meeting_time, instructor, prereq, metadata | ⬜ | Subset represented in CSV columns today. |
| Stable keys (term+subject+course+section) | 🟡 | |
| Normalize meeting times to intervals | 🟡 | Day mask + times in ingest + kernel. |

### 1.3 Data quality + drift monitoring

**Tech:** Great Expectations / dbt / SQL checks, Scheduler, alerting

| What we do | Progress | Notes |
|------------|----------|--------|
| Nulls, duplicates, invalid times, row-drop checks | 🟡 | `python3 -m optima_ingest.dq_check` (CI + scheduled workflow). |
| Drift alerts (e.g. section count drops) | 🟡 | Baseline file in DQ module; external alerting not wired. |

**Checkpoints (from your plan):** **Checkpoint A (end Week 2):** data pipeline on a **defined schedule** **+** **validated** silver schema **+** **minimal** DQ → **✅ closed** ([`.github/workflows/data-pipeline.yml`](../.github/workflows/data-pipeline.yml), `dq_check`, optional `bq_load` when GCP secrets are set).

---

## Phase 2: Optimization kernel MVP (Weeks 2–4)

**Tech:** Rust, Rayon, bitsets / interval indexing

| What we do | Progress | Notes |
|------------|----------|--------|
| Hard constraints: no time conflicts | ✅ | Pairwise-per-day **`detect_conflicts`**; Optimize final pass + DFS **bitset** pruning (minute maps per `day_code`). |
| Credit min/max, no classes before X, subjects | ✅ | **Total credit min/max** + **`prereq_groups`** JSON; **transitive** prereq satisfaction in Rust + Go legacy; optional **`make ingest-enrich`** for WesMaps-backed credits/prereqs. “Before X” via query param. *Prior credit / waivers not modeled.* |
| Pruning before enumeration | ✅ | DFS + caps + seeds + **weekly bitset** occupancy (1440 min/day) in Rust + Go legacy. |
| Parallel search + performance baseline | ✅ | **Rayon** (seeds, scoring, conflict scan). **Criterion** `optimize` bench + **`cargo bench --no-run`** in CI. *No numeric regression budget in CI yet—run `cargo bench` locally for comparisons.* |
| Multi-objective + Pareto | ✅ | Scoring + strict / epsilon Pareto in orchestrator + kernel `Optimize`. |

**Checkpoint B (end Week 4):** solver returns valid sets with hard constraints for the **MVP scope** above → **✅ closed** (see honest snapshot row for Phase 2).

---

## Phase 3: Orchestration layer (Weeks 4–5)

**Tech:** Go, gRPC, Protobuf, Buf (optional)

| What we do | Progress | Notes |
|------------|----------|--------|
| gRPC contract + service boundaries | 🟡 | `CheckConflicts`, `Optimize`, `Health`; richer “constraints + explanations” proto TBD. |
| API gateway + fallback reliability | 🟡 | gRPC Optimize + legacy fallback; no circuit breaker / Redis cache yet. |

**Checkpoint C (end Week 6):** end-to-end API + Pareto + fallback → **largely yes** for core path (minus enterprise polish).

---

## Phase 4: Intelligence layer v1 (Weeks 5–6)

**Tech:** DSPy / LangGraph, prompt eval

| What we do | Progress | Notes |
|------------|----------|--------|
| Intent → constraints / weights | ⬜ | |
| Guardrails + explainability | 🟡 | `debug=1` score breakdown on legacy path only. |

---

## Phase 5: Workload & uncertainty (Weeks 6–8)

**Tech:** BigQuery, embeddings, BQ ML / XGBoost

| What we do | Progress | Notes |
|------------|----------|--------|
| Workload signature pipeline | ⬜ | |
| Enrollment probability model | ⬜ | |

**Checkpoint D (end Week 8):** workload + uncertainty → **not started**.

---

## Phase 6: Observability, SLOs, scale (Weeks 8–9)

| What we do | Progress | Notes |
|------------|----------|--------|
| OTel across services | 🟡 | Go + Rust OTLP/HTTP; Jaeger [docker-compose](../docker-compose.yml). |
| Prometheus/Grafana, SLOs, load tests (k6) | ⬜ | |

---

## Phase 7: Product surface + UX (Weeks 9–10)

| What we do | Progress | Notes |
|------------|----------|--------|
| Next.js/React, Pareto viz, compare schedules | ⬜ | |

**Checkpoint E (end Week 10):** student UX + pilot-ready → **not started**.

---

## Phase 8: Pilot & iteration (Weeks 10–12)

| What we do | Progress | Notes |
|------------|----------|--------|
| Cohort pilot, analytics, A/B | ⬜ | |

**Checkpoint F (end Week 12):** pilot results + hardening report → **not started**.

---

## Cross-cutting standards

| Area | Progress |
|------|----------|
| Security & compliance (least privilege, PII, retention) | ⬜ Document in repo when deploying. |
| Testing (unit, integration, golden schedules) | 🟡 Rust unit tests; Go smoke; no golden regression suite yet. |
| Versioning (schema + protobuf) | 🟡 Proto in repo; no separate schema version field in CSV. |
| Failure design (timeouts, retries, fallback) | 🟡 gRPC timeouts + legacy fallback; not full matrix. |

---

## Technology roles (from your plan)

| Tech | Role in this repo today |
|------|-------------------------|
| **Go** | Orchestrator, HTTP API, gRPC client, OTel HTTP. |
| **Rust** | Kernel: conflicts, `Optimize` (credits + `prereq_groups` JSON from courses CSV, Rayon, Pareto), gRPC server. |
| **BigQuery** | **Silver** path wired (DDL + `bq_load`); **scheduled** GitHub workflow runs ingest + DQ; BQ load when secrets are configured. |
| **OpenTelemetry** | OTLP/HTTP, Jaeger locally. |
| **DSPy / LangGraph** | Future. |

---

## Risks & mitigations (tracking)

| Risk | Status |
|------|--------|
| HTML / source format changes | 🟡 Mitigate with snapshots + versioning when pipeline moves to cloud. |
| Solver complexity | 🟡 Staged features; **Criterion** bench exists; CI compiles it, no auto regression gate. |
| Sparse ML outcomes | ⬜ N/A until models. |
| AI invalid constraints | ⬜ Validator gate when NL layer exists. |

---

## Suggested next actions (pick order)

1. **Phase 3 (orchestration hardening):** circuit breaker, Redis/cache, richer proto (explanations in one response); optional **CI perf budget** for `cargo bench` if regressions become a risk.
2. **Tighten Phase 0:** optional KPI stub (Grafana folder) when you want visibility beyond Jaeger.
3. **Product / data depth:** prior completed-course credit in the solver; co-reqs / cross-list rules; stronger DQ or **buf**/**protoc** lint in CI.
4. **Phase 6:** Prometheus + SLOs when you have a persistent deployment.

---

*Footnote:* Day-to-day **status** belongs in issues/PRs/releases. **Edit this file** only when stakeholders change **phases, checkpoints, or scope** (see policy at top)—not for routine tick marks.
