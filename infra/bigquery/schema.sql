-- Optima WesMaps silver layer (aligned with python-ml CSV exports).
-- Create dataset first, e.g.:
--   bq --location=US mk -d "${GCP_PROJECT}:optima"
-- Then run this in BigQuery console or: bq query --use_legacy_sql=false < schema.sql

CREATE TABLE IF NOT EXISTS courses (
  term STRING NOT NULL OPTIONS(description = "WesMaps term code, e.g. 1269"),
  term_label STRING,
  subject_code STRING NOT NULL,
  course_code STRING NOT NULL,
  course_number STRING,
  course_title STRING,
  course_ref STRING,
  source_url STRING,
  ingested_at TIMESTAMP NOT NULL
)
PARTITION BY DATE(ingested_at)
CLUSTER BY term, subject_code, course_code;

CREATE TABLE IF NOT EXISTS sections (
  term STRING NOT NULL,
  term_label STRING,
  subject_code STRING NOT NULL,
  course_code STRING NOT NULL,
  course_ref STRING,
  section STRING NOT NULL,
  instructor STRING,
  meeting_pattern STRING,
  source_url STRING,
  ingested_at TIMESTAMP NOT NULL
)
PARTITION BY DATE(ingested_at)
CLUSTER BY term, subject_code, course_code, section;

CREATE TABLE IF NOT EXISTS meetings (
  term STRING NOT NULL,
  term_label STRING,
  subject_code STRING NOT NULL,
  course_code STRING NOT NULL,
  course_ref STRING,
  section STRING NOT NULL,
  day_code STRING NOT NULL,
  day_name STRING,
  start_time STRING,
  end_time STRING,
  source_url STRING,
  ingested_at TIMESTAMP NOT NULL
)
PARTITION BY DATE(ingested_at)
CLUSTER BY term, course_code, section, day_code;

CREATE TABLE IF NOT EXISTS ingest_runs (
  run_id STRING NOT NULL,
  term STRING NOT NULL,
  source STRING NOT NULL,
  started_at TIMESTAMP NOT NULL,
  finished_at TIMESTAMP,
  courses_rows INT64,
  sections_rows INT64,
  meetings_rows INT64,
  git_commit STRING
);
