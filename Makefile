.PHONY: run-orchestrator run-kernel run-ml venv ingest ingest-enrich dq pipeline gcs-bronze proto-go otel-jaeger otel-jaeger-down ci bq-load

# WesMaps term code (NOT the shell's TERM=xterm — use a dedicated name).
# Example: make ingest WES_TERM=1269 && make bq-load
WES_TERM ?= 1269

# Prefer python-ml/.venv after `make venv` (see .vscode/settings.json for the editor).
PYTHON_ML_SH = cd python-ml && if [ -x .venv/bin/python ]; then PY=.venv/bin/python; else PY=python3; fi

proto-go:
	cd go-orchestrator && export PATH="$$PATH:$$(go env GOPATH)/bin" && \
	protoc --go_out=. --go_opt=module=optima/go-orchestrator \
	  --go-grpc_out=. --go-grpc_opt=module=optima/go-orchestrator \
	  -I ../contracts/proto ../contracts/proto/optima/v1/kernel.proto

run-orchestrator:
	cd go-orchestrator && go run ./cmd/server

run-kernel:
	cd rust-kernel && cargo run

run-ml:
	$(PYTHON_ML_SH) && $$PY -m http.server 8888

# Create python-ml/.venv and install optima-python-ml + dev tools (pytest, ruff).
venv:
	cd python-ml && python3 -m venv .venv && .venv/bin/pip install -U pip && .venv/bin/pip install -e ".[dev]"

ingest:
	$(PYTHON_ML_SH) && $$PY -m pip install -q "requests>=2.31.0" "beautifulsoup4>=4.12.0" && \
	  PYTHONPATH=src $$PY -m optima_ingest.cli --term $(WES_TERM) --out-dir output

# Same as ingest + per-course WesMaps detail fetches (credits + prereq_groups JSON; ~1 HTTP call per distinct course_ref).
ingest-enrich:
	$(PYTHON_ML_SH) && $$PY -m pip install -q "requests>=2.31.0" "beautifulsoup4>=4.12.0" && \
	  PYTHONPATH=src $$PY -m optima_ingest.cli --term $(WES_TERM) --out-dir output --enrich

# Silver CSV DQ (Checkpoint A). Optional drift: DQ_DRIFT_MAX=0.35 make dq
dq:
	$(PYTHON_ML_SH) && $$PY -m pip install -q "requests>=2.31.0" "beautifulsoup4>=4.12.0" && \
	  PYTHONPATH=src $$PY -m optima_ingest.dq_check --term $(WES_TERM) --input-dir output \
	  $(if $(DQ_DRIFT_MAX),--drift-max-drop-ratio $(DQ_DRIFT_MAX),)

# ingest → dq → bq-load (set GCP_PROJECT; optional GCS_BRONZE_* after)
pipeline: ingest dq bq-load

# Upload output/bronze/<term>/ to GCS (requires GCS_BRONZE_BUCKET, ADC or gcloud auth)
gcs-bronze:
	@test -n "$(GCS_BRONZE_BUCKET)" || (echo "Set GCS_BRONZE_BUCKET=<bucket-name>" && exit 1)
	$(PYTHON_ML_SH) && $$PY -m pip install -q "google-cloud-storage>=2.14.0" && \
	  PYTHONPATH=src $$PY -m optima_ingest.gcs_bronze --term $(WES_TERM) --out-dir output \
	  --bucket "$(GCS_BRONZE_BUCKET)" --prefix "$(or $(GCS_BRONZE_PREFIX),bronze)"

# Jaeger UI http://localhost:16686 — OTLP/HTTP http://localhost:4318
otel-jaeger:
	docker compose up -d jaeger

otel-jaeger-down:
	docker compose down

# Local parity with .github/workflows/ci.yml (Rust + Go + Python pytest)
ci:
	cd rust-kernel && cargo test
	cd go-orchestrator && go test ./... -count=1
	cd python-ml && python3 -m pip install -q -e ".[dev]" && python3 -m pytest tests/ -q

# Install BigQuery client only (no editable install — avoids old pip + incomplete pyproject issues).
bq-load:
	$(PYTHON_ML_SH) && $$PY -m pip install -q "google-cloud-bigquery>=3.25.0" && \
	  PYTHONPATH=src $$PY -m optima_ingest.bq_load --term $(WES_TERM) --input-dir output
