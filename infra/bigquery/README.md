# BigQuery (silver layer)

1. **Auth:** `gcloud auth application-default login` (or a service account with BigQuery Data Editor + Job User on the project).

1b. **Reloads (no DML DELETE):** `optima_ingest.bq_load` loads CSVs into **staging** tables, then runs **`CREATE OR REPLACE TABLE ... AS`** (query job): rows for other terms are kept; the current term is replaced by the new file. That avoids **`DELETE`** (DML), which can fail with `403 billingNotEnabled` on some accounts even when billing is linked. Staging tables **`_optima_staging_courses`**, **`_optima_staging_sections`**, **`_optima_staging_meetings`** are created automatically.

1c. **Append-only mode:** Set **`BQ_SKIP_DELETE=1`** or **`--skip-delete`** to **append** without replacing the term (re-runs **duplicate** rows for that term).

1d. **ADC quota project (optional):** If unrelated API errors mention quota, try `gcloud auth application-default set-quota-project YOUR_PROJECT_ID`.

2. **Create dataset** (once):

   ```bash
   export GCP_PROJECT=your-project-id
   bq --location=US mk -d "${GCP_PROJECT}:optima"
   ```

   Or use another dataset name and set **`BQ_DATASET`** when loading.

3. **Create tables:** paste/run `schema.sql` in the [BigQuery console](https://console.cloud.google.com/bigquery) (SQL workspace), or split into one statement per table if the UI requires it.

   **Existing `sections` tables:** if you created them before the `credits` column existed, add it before loading newer CSVs, for example:  
   `ALTER TABLE sections ADD COLUMN IF NOT EXISTS credits FLOAT64;`

   **Existing `courses` tables:** add `prereq_groups` if missing:  
   `ALTER TABLE courses ADD COLUMN IF NOT EXISTS prereq_groups STRING;`

4. **Load from local CSVs** (after `make ingest`):

   ```bash
   cd python-ml
   python3 -m pip install "google-cloud-bigquery>=3.25.0"
   export GCP_PROJECT=your-project-id
   export BQ_DATASET=optima   # optional; default is optima
   PYTHONPATH=src python3 -m optima_ingest.bq_load --term 1269 --input-dir output
   ```

   From repo root you can use **`make bq-load`** (uses **`WES_TERM`**, default `1269` — **not** `TERM`, which is the terminal type `xterm-256color` and will break loads).

   This **replaces** rows for that `term` (via staging + `CREATE OR REPLACE TABLE ... AS`, not DML `DELETE`), and records a row in **`ingest_runs`**.

Bronze HTML remains on disk (or future GCS); this path is **CSV → BigQuery** only.

## Checkpoint A (roadmap): schedule + DQ + optional GCS bronze

- **Local one-shot:** `make pipeline` (ingest → `make dq` → `make bq-load`). Set **`GCP_PROJECT`** for BigQuery.
- **DQ only:** `make dq` on existing `python-ml/output/` CSVs. Optional drift: `DQ_DRIFT_MAX=0.35 make dq` writes/compares `output/.dq_baseline_<term>.json`.
- **GCS bronze (optional):** create a bucket, then `GCS_BRONZE_BUCKET=your-bucket make gcs-bronze` (requires `google-cloud-storage` and ADC; bronze dir must exist — run ingest **without** `--no-bronze`).
- **GitHub Actions:** [`.github/workflows/data-pipeline.yml`](../../.github/workflows/data-pipeline.yml) runs **daily** (06:00 UTC) + **workflow_dispatch**. Add secrets **`GCP_PROJECT`**, **`GCP_SA_JSON`** to enable **`bq-load`** in CI. Optional Repository **Variable** **`WES_TERM`**.
- **BQ-side checks:** [`dq/row_sanity.sql`](dq/row_sanity.sql) (edit project/dataset/term).
