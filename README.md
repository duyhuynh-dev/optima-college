# Optima

Distributed academic optimization framework for course schedule optimization.

## Repository Layout

- `go-orchestrator/` - Go API gateway and orchestration service
- `rust-kernel/` - Rust optimization kernel
- `python-ml/` - data ingestion and ML experimentation workspace
- `contracts/` - shared contracts and schemas
- `scripts/` - utility scripts and local bootstrap

## Quick Start

1. Start Rust kernel (HTTP on `:8090`, **gRPC on `:50051`**):
   - `make run-kernel`
2. In another terminal, start Go orchestrator:
   - `make run-orchestrator`
3. Test request:
   - `curl -s http://localhost:8080/v1/schedules`
4. Optional: legacy HTTP conflict check on kernel:
   - `curl -s "http://localhost:8090/v1/conflicts?sections=COMP112-01,COMP112-02"`

The orchestrator **prefers gRPC** (`Kernel.CheckConflicts`) at `KERNEL_GRPC_ADDR` (default `localhost:50051`) and falls back to HTTP if gRPC is unavailable. Regenerate Go stubs after editing protos: `make proto-go`.

With both services running, `/v1/schedules` calls the Rust kernel and filters out conflicting options.
The orchestrator now generates candidate schedules from `python-ml/output/sections_1269.csv`.
You can tune generation with query params, for example:
- `curl -s "http://localhost:8080/v1/schedules?k=4&max_results=10&earliest_start=10:00AM"`
- `curl -s "http://localhost:8080/v1/schedules?k=4&max_results=10&max_per_subject=1&subject_whitelist=COMP,ECON&subject_blacklist=AFAM"`

Hard **credit load** (Phase 2): optional `min_total_credits` and `max_total_credits` bound the sum of per-row `credits` in the sections CSV (ingest writes `1.0` by default until real credit values are parsed). Re-run `make ingest` if your CSV predates the `credits` column.

**Prerequisites** (Phase 2): `courses_<term>.csv` column **`prereq_groups`** — JSON array of OR-clauses, AND across clauses (e.g. `[["MATH120","MATH121"]]` means at least one of those codes if the student takes the course). **`make ingest`** writes `[]`; run **`make ingest-enrich`** (many HTTP requests) to fill credits + prereqs from WesMaps course-detail pages.

`expected_utility` and `stress_score` are computed from `python-ml/output/meetings_1269.csv`: weekly contact time, evening load (starts from 5:00 PM), early-morning load (before 9:00 AM), back-to-back blocks (gap under 12 minutes), and busiest day. Utility is `1 - stress` after blending those signals (caps normalize each term to roughly 0–1).

Tune weights (non-negative; server normalizes to sum to 1): `w_weekly`, `w_evening`, `w_early`, `w_back_to_back`, `w_busy_day`. Example: prioritize avoiding night classes: `&w_evening=0.6&w_weekly=0.2&w_early=0.1&w_back_to_back=0.05&w_busy_day=0.05`.

Add `debug=1` to include `score_breakdown` on each option and see `score_weights` in the response.
Add `pareto=1` to return only non-dominated schedules by maximizing `expected_utility` and minimizing `stress_score`.
Use `pareto_mode=epsilon&pareto_epsilon=0.03` (or rely on strict auto-fallback) to broaden tradeoff choices when strict Pareto returns too few options.

### AI agent plan (natural language → structured intent → schedules)

- **Endpoint:** `POST http://localhost:8080/v1/agent/plan` with JSON body `{"message":"...","locale":"en"}`.
- **Secrets:** set **`OPENAI_API_KEY`**; optional **`OPENAI_MODEL`** (default `gpt-4o-mini`).
- **Response:** `structured_panel` (validated `schedule_intent_v1`), optional **`schedules`** (same shape as `/v1/schedules`), and **`schedule_query`** echoing the internal allowlisted call.
- **Contracts:** versioned schema and policy in [`contracts/agent/README.md`](contracts/agent/README.md) (Go embeds a duplicate at `go-orchestrator/internal/agent/schedule_intent_v1.schema.json` — keep in sync).
- **Optional:** `ORCHESTRATOR_AGENT_CATALOG_STATS` — single-line aggregate hint only (no row-level catalog in prompts).

Example:

```bash
curl -s -X POST http://localhost:8080/v1/agent/plan \
  -H "Content-Type: application/json" \
  -d '{"message":"I want 4 courses, avoid evening classes, subject areas COMP and ECON","locale":"en"}'
```

## Data Ingestion (Phase 1)

**Python venv (recommended):** run **`make venv`** once to create `python-ml/.venv` and install the package with dev tools (`pytest`, `ruff`). The workspace [`.vscode/settings.json`](.vscode/settings.json) points Cursor/VS Code at that interpreter. **`make ingest`**, **`make dq`**, **`make bq-load`**, and related targets use `.venv/bin/python` automatically when the venv exists; otherwise they fall back to `python3`.

Run initial public WesMaps ingest:

- `make ingest`

This creates normalized CSV outputs under `python-ml/output/`, and **bronze** raw HTML snapshots under `python-ml/output/bronze/<term>/` (index + one file per subject page) for replay and parser versioning. Use `python -m optima_ingest.cli --term <TERM> --out-dir output --no-bronze` to skip bronze and save disk.

**BigQuery (silver → warehouse):** DDL in [`infra/bigquery/schema.sql`](infra/bigquery/schema.sql), operator steps in [`infra/bigquery/README.md`](infra/bigquery/README.md). After `gcloud auth` and creating the dataset/tables, run **`make bq-load`** (set **`GCP_PROJECT`**, optional **`BQ_DATASET`**, **`WES_TERM`** — e.g. `1269`; do **not** use the name `TERM`, it conflicts with the terminal’s `TERM=xterm-256color`).

**Checkpoint A (roadmap):** **`make dq`** validates silver CSVs; **`make pipeline`** runs ingest → dq → bq-load. Scheduled/GitHub setup: [`infra/bigquery/README.md`](infra/bigquery/README.md) § Checkpoint A.

See **`CONTRIBUTING.md`** and the program plan in **`docs/ROADMAP.md`** (treat that file as baseline; do not edit it for routine progress).

## OpenTelemetry (optional)

**Go** and **Rust** both use **OTLP over HTTP** to the same endpoint. Set:

```bash
export OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318
```

Optionally set **`OTEL_SERVICE_NAME`** (defaults: `optima-orchestrator`, `optima-rust-kernel`).

**Local Jaeger** (UI at [http://localhost:16686](http://localhost:16686), OTLP HTTP on **:4318**):

```bash
make otel-jaeger
export OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318
make run-kernel    # terminal 1
make run-orchestrator   # terminal 2
```

Stop Jaeger: `make otel-jaeger-down`. You can point the same env vars at any OTLP/HTTP-compatible collector or cloud endpoint.

**Go** instruments HTTP with `otelhttp` ( **`/health` is not traced** to reduce noise). **Rust** attaches an OTLP exporter + `tracing` when `OTEL_EXPORTER_OTLP_ENDPOINT` is set; otherwise it logs with `RUST_LOG` / `tracing-subscriber` env filter.

## Status & roadmap

**Full phased plan (Phases 0–8), checkpoints, and progress tracking:** [`docs/ROADMAP.md`](docs/ROADMAP.md).

| Area | Done | Next |
|------|------|------|
| **Data** | WesMaps → CSV ingest (`make ingest`), sections/meetings for scoring, BQ load path | **`make pipeline`** + DQ (`make dq`); daily job in [`.github/workflows/data-pipeline.yml`](.github/workflows/data-pipeline.yml) |
| **Rust kernel** | HTTP conflicts, gRPC `Optimize` (credits + direct prereqs, Rayon scoring / conflict scan / seeds), OTLP traces | Optional: more unit tests around `optimize` |
| **Go orchestrator** | `/v1/schedules` (prefers kernel `Optimize` via gRPC; `legacy=1` fallback), OTLP, smoke tests | More integration tests (with kernel / fixtures) |
| **Contracts** | `kernel.proto` (`CheckConflicts`, `Optimize`, `Health`) | Evolve as new RPCs are added |
| **Observability** | Jaeger `docker-compose`, shared OTLP/HTTP `:4318` | Production collector / dashboards |
| **CI** | [GitHub Actions](.github/workflows/ci.yml): `cargo test` + `go test` on push/PR | Optional: Python ingest in CI, `buf`/`protoc` lint for protos |

**Where we are:** core path is working end-to-end (ingest → kernel + orchestrator → schedules). CI guards Rust and Go tests on every push/PR. **`TestSchedulesHandler_Smoke`** skips if CSVs are missing (run `make ingest` locally for full coverage).

**Suggested next focus (roadmap):** finish **Checkpoint A** — wire **GitHub secrets** for `data-pipeline.yml` if you want automated BQ loads; add **GCS bronze** and stronger **DQ** as needed.

## Next engineering steps (detail)

- Enable **`GCP_PROJECT`** + **`GCP_SA_JSON`** on the repo for scheduled **`bq-load`**; tune **`DQ_DRIFT_MAX`** locally; optional **GCS** bucket for bronze
