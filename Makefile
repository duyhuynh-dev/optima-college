.PHONY: run-orchestrator run-kernel run-ml ingest dq pipeline gcs-bronze proto-go otel-jaeger otel-jaeger-down ci bq-load

# WesMaps term code (NOT the shell's TERM=xterm — use a dedicated name).
# Example: make ingest WES_TERM=1269 && make bq-load
WES_TERM ?= 1269

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
	cd python-ml && python3 -m http.server 8888

ingest:
	cd python-ml && python3 -m pip install -q "requests>=2.31.0" "beautifulsoup4>=4.12.0" && \
	  PYTHONPATH=src python3 -m optima_ingest.cli --term $(WES_TERM) --out-dir output

# Silver CSV DQ (Checkpoint A). Optional drift: DQ_DRIFT_MAX=0.35 make dq
dq:
	cd python-ml && python3 -m pip install -q "requests>=2.31.0" "beautifulsoup4>=4.12.0" && \
	  PYTHONPATH=src python3 -m optima_ingest.dq_check --term $(WES_TERM) --input-dir output \
	  $(if $(DQ_DRIFT_MAX),--drift-max-drop-ratio $(DQ_DRIFT_MAX),)

# ingest → dq → bq-load (set GCP_PROJECT; optional GCS_BRONZE_* after)
pipeline: ingest dq bq-load

# Upload output/bronze/<term>/ to GCS (requires GCS_BRONZE_BUCKET, ADC or gcloud auth)
gcs-bronze:
	@test -n "$(GCS_BRONZE_BUCKET)" || (echo "Set GCS_BRONZE_BUCKET=<bucket-name>" && exit 1)
	cd python-ml && python3 -m pip install -q "google-cloud-storage>=2.14.0" && \
	  PYTHONPATH=src python3 -m optima_ingest.gcs_bronze --term $(WES_TERM) --out-dir output \
	  --bucket "$(GCS_BRONZE_BUCKET)" --prefix "$(or $(GCS_BRONZE_PREFIX),bronze)"

# Jaeger UI http://localhost:16686 — OTLP/HTTP http://localhost:4318
otel-jaeger:
	docker compose up -d jaeger

otel-jaeger-down:
	docker compose down

# Local parity with .github/workflows/ci.yml
ci:
	cd rust-kernel && cargo test
	cd go-orchestrator && go test ./... -count=1

# Install BigQuery client only (no editable install — avoids old pip + incomplete pyproject issues).
bq-load:
	cd python-ml && python3 -m pip install -q "google-cloud-bigquery>=3.25.0" && \
	  PYTHONPATH=src python3 -m optima_ingest.bq_load --term $(WES_TERM) --input-dir output
