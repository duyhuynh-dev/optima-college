# Contributing to Optima

## Before opening a PR

1. From the repo root, run **`make ci`** (`cargo test` in `rust-kernel/`, `go test` in `go-orchestrator/`).
2. If you changed **`contracts/proto/**/*.proto`**, regenerate Go stubs: **`make proto-go`**, then commit the generated files under `go-orchestrator/internal/gen/`.
3. If you changed the WesMaps ingest, run **`make ingest`** (or **`make ingest-enrich`** for detail-backed credits + `prereq_groups`) and sanity-check `python-ml/output/`. **`make dq`** expects **`credits`** on sections and **`prereq_groups`** JSON on courses.

## Conventions

- **Branches:** use descriptive names (`feature/…`, `fix/…`, `chore/…`).
- **Commits:** clear, imperative subject lines; optional body for context.
- **API contracts:** keep `contracts/proto` as the source of truth; avoid ad-hoc JSON that duplicates proto fields without a documented reason.

## Local setup

- **Rust:** stable toolchain, `protobuf-compiler` for `rust-kernel` (`tonic-build`).
- **Go:** version per `go-orchestrator/go.mod`.
- **Python:** `python-ml` uses a venv; install with `pip install -e ".[dev]"` or your project’s `pyproject.toml` dev deps.
- **BigQuery load:** `make bq-load` installs `google-cloud-bigquery` into the active `python3` and runs the loader with `PYTHONPATH=src` (or `python3 -m pip install -e ".[bq]"` after a modern `pip` if you want the full editable package). Requires **`GCP_PROJECT`**. Use **`WES_TERM=1269`** (not **`TERM`**).

## Operational KPIs (target dashboard)

Track toward production (not all wired yet): **p95 latency**, **schedule validity rate**, **catalog freshness**, **service uptime**. See OpenTelemetry + Jaeger in the root `README.md`.

## Program plan

The phased roadmap lives in **`docs/ROADMAP.md`**. Per its document policy, **do not edit that file** for routine progress—only after explicit plan revisions with stakeholders.
