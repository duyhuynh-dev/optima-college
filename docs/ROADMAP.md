# Optima — phased plan & progress

> **Document policy (stable):** This file is the agreed **program baseline**. Do not edit it for routine implementation updates, progress ticks, or “sync with main” housekeeping. Revise it only after **explicit stakeholder consultation** when the plan itself changes (phases, checkpoints, or scope). Track execution in issues, PRs, or release notes—not here.

**Why this exists:** A single, detailed roadmap so work stays **ordered from start to finish** (phases → checkpoints). **Operational runbooks** live in [`README.md`](../README.md), [`CONTRIBUTING.md`](../CONTRIBUTING.md), and [`infra/`](../infra/); **scope and sequencing** live here.

**Execution discipline:** Follow the phased plan; **bugs and breakages** are fixed and then work returns to the current phase/checkpoint—no ad hoc rescoping without updating this document under the policy above.

This document is the **structured program plan**. Last reviewed: 2026-04-07.

**Legend:** ✅ Done · 🟡 Partial / in flight · ⬜ Not started

**Critical path (dependency order):** Phase **1** data foundation → **2–3** kernel + orchestration (vertical slice) → **4+** intelligence/product/pilot. Later phases assume earlier checkpoints where noted.

---

## How the current repo maps (honest snapshot)

| Your phase | Match today |
|------------|-------------|
| **0** | 🟡 Repo layout (`go-orchestrator`, `rust-kernel`, `python-ml`, `contracts`), `Makefile`, CI, protobuf, local Jaeger + OTLP. Missing: issue tracker choice, PR templates, Terraform, KPI dashboard skeleton. |
| **1** | 🟡 **1.1** Ingest → CSV + optional **local bronze** HTML under `python-ml/output/bronze/`; **silver** in **BigQuery** via [`infra/bigquery/`](../infra/bigquery/) + `make bq-load` (manual path). **GCS bronze** + **scheduled** pipeline not done. **1.2** Practical IDs in CSV + meetings normalized for solving; not full entity DDL everywhere. **1.3** DQ/drift alerts not built. |
| **2** | 🟡 Conflicts + scoring + Pareto-style tradeoffs exist; search is **combinatorial + filters**, not a full **Rayon/bitset** constraint engine with credits as hard constraints. |
| **3** | 🟡 gRPC `CheckConflicts` + `Optimize`, Go gateway, degraded **legacy** path. Missing: richer proto (weights/explanations in one shot was partially done), circuit breaker, Redis. |
| **4–8** | ⬜ Not started (NL intent, workload ML, frontend, pilot). |

**Verdict:** The architecture you sketched **matches the direction** of what we built (data → Rust kernel → Go API → observability). **Depth** vs the doc: we are **lighter on cloud bronze, formal DQ, and a full constraint/solver engine** than Phases 1–2 describe—we prioritized **vertical slice** (ingest → optimize → API) first.

---

## Phase 0: Program Setup (Week 0)

**Tech:** GitHub, Linear/Jira/Notion, Docker, Makefile, OpenTelemetry collector, Terraform (optional)

| What we do | Progress | Notes |
|------------|----------|--------|
| Mono-repo structure (`go-orchestrator`, `rust-kernel`, data/ML, contracts, infra) | ✅ | `python-ml/` = data pipeline; no separate `data-pipeline/` repo. |
| Engineering standards: branching, PR template, testing gates, release tags | 🟡 | CI: [`.github/workflows/ci.yml`](../.github/workflows/ci.yml) (`cargo test`, `go test`). PR template / branching TBD. |
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
| Bronze: raw HTML snapshots | 🟡 | Local bronze HTML optional (`--no-bronze` to skip). **GCS (or cloud) bronze** ⬜. |
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
| Nulls, duplicates, invalid times, row-drop checks | ⬜ | |
| Drift alerts (e.g. section count drops) | ⬜ | |

**Checkpoints (from your plan):** **Checkpoint A (end Week 2):** data pipeline on a **defined schedule** (e.g. daily) **+** **validated** silver schema **+** **minimal** DQ (nulls / row sanity / drift hooks as agreed) → **not yet** (manual ingest + BQ load today; scheduler + DQ outstanding).

---

## Phase 2: Optimization kernel MVP (Weeks 2–4)

**Tech:** Rust, Rayon, bitsets / interval indexing

| What we do | Progress | Notes |
|------------|----------|--------|
| Hard constraints: no time conflicts | ✅ | Kernel conflict detection + Optimize filters conflict-free sets. |
| Credit min/max, no classes before X, subjects | 🟡 | “Before X” via query param; credits/prereqs not full solver constraints. |
| Pruning before enumeration | 🟡 | DFS + caps + seeds; not bitset/Rayon engine. |
| Parallel search + performance baseline | ⬜ | |
| Multi-objective + Pareto | 🟡 | Scoring + Pareto / epsilon in orchestrator + `Optimize`. |

**Checkpoint B (end Week 4):** solver returns valid sets with hard constraints → **partially** (conflicts yes; full credit/prereq hard constraints not yet).

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
| **Rust** | Kernel: conflicts, scoring pipeline in `Optimize`, gRPC server. |
| **BigQuery** | **Silver** path wired (DDL + `bq_load` from CSV); **scheduled** loads + **DQ** still roadmap Phase 1. |
| **OpenTelemetry** | OTLP/HTTP, Jaeger locally. |
| **DSPy / LangGraph** | Future. |

---

## Risks & mitigations (tracking)

| Risk | Status |
|------|--------|
| HTML / source format changes | 🟡 Mitigate with snapshots + versioning when pipeline moves to cloud. |
| Solver complexity | 🟡 Staged features; benchmarks TBD. |
| Sparse ML outcomes | ⬜ N/A until models. |
| AI invalid constraints | ⬜ Validator gate when NL layer exists. |

---

## Suggested next actions (pick order)

1. **Close Checkpoint A (Phase 1):** **schedule** ingest + warehouse load; add **minimal DQ** (SQL or scripts); optional **GCS bronze** for replay/versioning.
2. **Tighten Phase 0:** PR template + `CONTRIBUTING.md` branching; optional KPI stub (even Grafana folder).
3. **Deepen Phase 2:** explicit hard constraints (credits, prereqs) in kernel vs heuristics only.
4. **Phase 6:** Prometheus + SLOs when you have a persistent deployment.

---

*Footnote:* Day-to-day **status** belongs in issues/PRs/releases. **Edit this file** only when stakeholders change **phases, checkpoints, or scope** (see policy at top)—not for routine tick marks.
