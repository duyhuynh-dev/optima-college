"""
Load WesMaps CSV outputs (silver) into BigQuery tables defined in infra/bigquery/schema.sql.

Requires: pip install 'optima-python-ml[bq]' (google-cloud-bigquery).

Environment:
  GCP_PROJECT or GOOGLE_CLOUD_PROJECT — GCP project id
  BQ_DATASET — dataset id (default: optima)
  BQ_SKIP_DELETE=1 — append-only load (no term replace; re-runs duplicate rows)

By default, reloads are **idempotent without DML DELETE**: data is loaded into a staging
table, then the main table is replaced with
``CREATE OR REPLACE TABLE ... AS (old rows WHERE term != @term UNION ALL staging)``.
That avoids BigQuery **DELETE** (DML), which can fail with ``billingNotEnabled`` on some
accounts even when billing is linked.

Example:
  export GCP_PROJECT=my-project
  export BQ_DATASET=optima
  python -m optima_ingest.bq_load --term 1269 --input-dir output
"""

from __future__ import annotations

import argparse
import csv
import io
import os
import subprocess
import sys
import tempfile
import uuid
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from google.cloud import bigquery
from google.cloud.exceptions import NotFound


def _project_id() -> str:
    return os.environ.get("GCP_PROJECT") or os.environ.get("GOOGLE_CLOUD_PROJECT") or ""


def _log_project_resolution(project: str) -> None:
    """Help debug wrong project id."""
    gcp = os.environ.get("GCP_PROJECT", "").strip()
    gcloud = os.environ.get("GOOGLE_CLOUD_PROJECT", "").strip()
    if gcp and gcloud and gcp != gcloud:
        print(
            f"Warning: GCP_PROJECT={gcp!r} and GOOGLE_CLOUD_PROJECT={gcloud!r} "
            f"differ; using {project!r} (GCP_PROJECT wins when set). "
            "Unset the other if BigQuery errors persist.",
            file=sys.stderr,
        )
    print(f"[bq_load] BigQuery project: {project!r}", file=sys.stderr)


def _dataset_id() -> str:
    return os.environ.get("BQ_DATASET", "optima")


def _skip_delete_from_env() -> bool:
    return os.environ.get("BQ_SKIP_DELETE", "").strip().lower() in ("1", "true", "yes")


def _git_commit() -> str:
    try:
        return (
            subprocess.check_output(
                ["git", "rev-parse", "--short", "HEAD"],
                cwd=Path(__file__).resolve().parents[3],
                stderr=subprocess.DEVNULL,
            )
            .decode()
            .strip()
        )
    except (subprocess.CalledProcessError, FileNotFoundError, OSError):
        return ""


def _fq(client: bigquery.Client, table: str) -> str:
    return f"`{client.project}.{_dataset_id()}.{table}`"


# Match infra/bigquery/schema.sql — CSV column order matches dataclass field order + ingested_at.
# Autodetect is unsafe: numeric-looking strings (e.g. term "1269") become INT64 and break UNION ALL.
_SCHEMA_COURSES: list[bigquery.SchemaField] = [
    bigquery.SchemaField("term", "STRING"),
    bigquery.SchemaField("term_label", "STRING"),
    bigquery.SchemaField("subject_code", "STRING"),
    bigquery.SchemaField("course_code", "STRING"),
    bigquery.SchemaField("course_number", "STRING"),
    bigquery.SchemaField("course_title", "STRING"),
    bigquery.SchemaField("course_ref", "STRING"),
    bigquery.SchemaField("prereq_groups", "STRING"),
    bigquery.SchemaField("source_url", "STRING"),
    bigquery.SchemaField("ingested_at", "TIMESTAMP"),
]
_SCHEMA_SECTIONS: list[bigquery.SchemaField] = [
    bigquery.SchemaField("term", "STRING"),
    bigquery.SchemaField("term_label", "STRING"),
    bigquery.SchemaField("subject_code", "STRING"),
    bigquery.SchemaField("course_code", "STRING"),
    bigquery.SchemaField("course_ref", "STRING"),
    bigquery.SchemaField("section", "STRING"),
    bigquery.SchemaField("instructor", "STRING"),
    bigquery.SchemaField("meeting_pattern", "STRING"),
    bigquery.SchemaField("credits", "FLOAT64"),
    bigquery.SchemaField("source_url", "STRING"),
    bigquery.SchemaField("ingested_at", "TIMESTAMP"),
]
_SCHEMA_MEETINGS: list[bigquery.SchemaField] = [
    bigquery.SchemaField("term", "STRING"),
    bigquery.SchemaField("term_label", "STRING"),
    bigquery.SchemaField("subject_code", "STRING"),
    bigquery.SchemaField("course_code", "STRING"),
    bigquery.SchemaField("course_ref", "STRING"),
    bigquery.SchemaField("section", "STRING"),
    bigquery.SchemaField("day_code", "STRING"),
    bigquery.SchemaField("day_name", "STRING"),
    bigquery.SchemaField("start_time", "STRING"),
    bigquery.SchemaField("end_time", "STRING"),
    bigquery.SchemaField("source_url", "STRING"),
    bigquery.SchemaField("ingested_at", "TIMESTAMP"),
]

_SCHEMA_BY_MAIN_TABLE: dict[str, list[bigquery.SchemaField]] = {
    "courses": _SCHEMA_COURSES,
    "sections": _SCHEMA_SECTIONS,
    "meetings": _SCHEMA_MEETINGS,
}


def _load_staging(
    client: bigquery.Client,
    staging_table: str,
    csv_path: Path,
    schema: list[bigquery.SchemaField],
) -> None:
    """Load CSV into a staging table, replacing any previous content."""
    table_ref = f"{client.project}.{_dataset_id()}.{staging_table}"
    # Drop if a prior run used autodetect (wrong types); load recreates with `schema`.
    client.delete_table(table_ref, not_found_ok=True)
    job_config = bigquery.LoadJobConfig(
        source_format=bigquery.SourceFormat.CSV,
        skip_leading_rows=1,
        schema=schema,
        autodetect=False,
        write_disposition=bigquery.WriteDisposition.WRITE_TRUNCATE,
    )
    with csv_path.open("rb") as f:
        job = client.load_table_from_file(f, table_ref, job_config=job_config)
    job.result()


def _load_append(
    client: bigquery.Client,
    table: str,
    csv_path: Path,
    schema: list[bigquery.SchemaField],
) -> None:
    table_ref = f"{client.project}.{_dataset_id()}.{table}"
    job_config = bigquery.LoadJobConfig(
        source_format=bigquery.SourceFormat.CSV,
        skip_leading_rows=1,
        schema=schema,
        autodetect=False,
        write_disposition=bigquery.WriteDisposition.WRITE_APPEND,
    )
    with csv_path.open("rb") as f:
        job = client.load_table_from_file(f, table_ref, job_config=job_config)
    job.result()


def _replace_term_via_ctas(
    client: bigquery.Client,
    term: str,
    main_table: str,
    staging_table: str,
    cluster_by: str,
    select_cols: str,
) -> None:
    """
    Replace main_table with (rows where term != @term) UNION ALL staging.
    Uses CREATE OR REPLACE TABLE (query job), not DML DELETE.
    """
    main = _fq(client, main_table)
    staging = _fq(client, staging_table)
    query = f"""
CREATE OR REPLACE TABLE {main}
PARTITION BY DATE(ingested_at)
CLUSTER BY {cluster_by}
AS
SELECT {select_cols}
FROM {main}
WHERE term != @term
UNION ALL
SELECT {select_cols}
FROM {staging}
"""
    job_config = bigquery.QueryJobConfig(
        query_parameters=[bigquery.ScalarQueryParameter("term", "STRING", term)],
    )
    client.query(query, job_config=job_config).result()


# Explicit column lists — must match infra/bigquery/schema.sql and CSV headers.
_COURSES_COLS = (
    "term, term_label, subject_code, course_code, course_number, course_title, "
    "course_ref, prereq_groups, source_url, ingested_at"
)
_SECTIONS_COLS = (
    "term, term_label, subject_code, course_code, course_ref, section, instructor, "
    "meeting_pattern, credits, source_url, ingested_at"
)
_MEETINGS_COLS = (
    "term, term_label, subject_code, course_code, course_ref, section, day_code, "
    "day_name, start_time, end_time, source_url, ingested_at"
)

_TABLE_SPECS: tuple[tuple[str, str, str, str, str], ...] = (
    ("courses", "_optima_staging_courses", "term, subject_code, course_code", _COURSES_COLS),
    ("sections", "_optima_staging_sections", "term, subject_code, course_code, section", _SECTIONS_COLS),
    ("meetings", "_optima_staging_meetings", "term, course_code, section, day_code", _MEETINGS_COLS),
)


def _augment_csv(
    src: Path,
    out: Path,
    ingested_at: datetime,
) -> int:
    """Write CSV with ingested_at column; return data row count."""
    with src.open(newline="", encoding="utf-8") as f_in:
        reader = csv.DictReader(f_in)
        rows = list(reader)
        fieldnames = list(reader.fieldnames or [])
        if "ingested_at" in fieldnames:
            raise ValueError(f"{src} already has ingested_at")
        fieldnames = fieldnames + ["ingested_at"]
        out.parent.mkdir(parents=True, exist_ok=True)
        with out.open("w", encoding="utf-8", newline="") as f_out:
            w = csv.DictWriter(f_out, fieldnames=fieldnames)
            w.writeheader()
            iso = ingested_at.replace(tzinfo=timezone.utc).isoformat()
            for row in rows:
                row["ingested_at"] = iso
                w.writerow(row)
    return len(rows)


def _ensure_dataset(client: bigquery.Client) -> None:
    ds = _dataset_id()
    ref = f"{client.project}.{ds}"
    try:
        client.get_dataset(ref)
    except NotFound:
        raise SystemExit(
            f"BigQuery dataset not found: {ref}. Create it first, e.g.:\n"
            f'  bq --location=US mk -d "{ref}"\n'
            "Then apply infra/bigquery/schema.sql (tables)."
        ) from None


def _append_ingest_run(client: bigquery.Client, ir: dict[str, Any]) -> None:
    """Record run metadata; prefer streaming insert, fall back to load job (one row CSV)."""
    table_id = f"{client.project}.{_dataset_id()}.ingest_runs"
    errors = client.insert_rows_json(table_id, [ir])
    if not errors:
        return
    # Fallback: append via load job (no DML / streaming).
    buf = io.StringIO()
    w = csv.DictWriter(
        buf,
        fieldnames=[
            "run_id",
            "term",
            "source",
            "started_at",
            "finished_at",
            "courses_rows",
            "sections_rows",
            "meetings_rows",
            "git_commit",
        ],
    )
    w.writeheader()
    w.writerow(ir)
    data = buf.getvalue().encode("utf-8")
    job_config = bigquery.LoadJobConfig(
        source_format=bigquery.SourceFormat.CSV,
        skip_leading_rows=1,
        autodetect=True,
        write_disposition=bigquery.WriteDisposition.WRITE_APPEND,
    )
    job = client.load_table_from_file(io.BytesIO(data), table_id, job_config=job_config)
    job.result()


def run_load(
    term: str,
    input_dir: Path,
    *,
    skip_delete: bool = False,
) -> dict[str, Any]:
    project = _project_id()
    if not project:
        raise SystemExit(
            "Set GCP_PROJECT or GOOGLE_CLOUD_PROJECT to your GCP project id.",
        )
    _log_project_resolution(project)

    ingested_at = datetime.now(timezone.utc)
    run_id = str(uuid.uuid4())
    started = ingested_at

    client = bigquery.Client(project=project)
    _ensure_dataset(client)

    courses_csv = input_dir / f"courses_{term}.csv"
    sections_csv = input_dir / f"sections_{term}.csv"
    meetings_csv = input_dir / f"meetings_{term}.csv"
    for p in (courses_csv, sections_csv, meetings_csv):
        if not p.exists():
            raise FileNotFoundError(f"Missing {p} — run ingest first.")

    counts: dict[str, int] = {}
    with tempfile.TemporaryDirectory() as tmp:
        tdir = Path(tmp)
        csv_by_logical = {
            "courses": courses_csv,
            "sections": sections_csv,
            "meetings": meetings_csv,
        }
        for main_table, staging_table, cluster_by, select_cols in _TABLE_SPECS:
            logical = main_table
            src = csv_by_logical[logical]
            aug = tdir / f"{logical}.csv"
            counts[logical] = _augment_csv(src, aug, ingested_at)
            schema = _SCHEMA_BY_MAIN_TABLE[logical]
            if skip_delete:
                _load_append(client, main_table, aug, schema)
            else:
                _load_staging(client, staging_table, aug, schema)
                _replace_term_via_ctas(
                    client,
                    term,
                    main_table,
                    staging_table,
                    cluster_by,
                    select_cols,
                )

    finished = datetime.now(timezone.utc)

    ir = {
        "run_id": run_id,
        "term": term,
        "source": "wesmaps_csv",
        "started_at": started.isoformat(),
        "finished_at": finished.isoformat(),
        "courses_rows": counts["courses"],
        "sections_rows": counts["sections"],
        "meetings_rows": counts["meetings"],
        "git_commit": _git_commit(),
    }
    _append_ingest_run(client, ir)

    return {"run_id": run_id, **counts}


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(description="Load WesMaps CSVs into BigQuery.")
    p.add_argument("--term", required=True, help="Term code, e.g. 1269")
    p.add_argument(
        "--input-dir",
        type=Path,
        default=Path("output"),
        help="Directory containing courses_<term>.csv etc.",
    )
    p.add_argument(
        "--skip-delete",
        action="store_true",
        help="Append-only (no term replace); re-runs duplicate rows for that term.",
    )
    return p


def main() -> None:
    args = build_parser().parse_args()
    skip = bool(args.skip_delete or _skip_delete_from_env())
    result = run_load(term=args.term, input_dir=args.input_dir, skip_delete=skip)
    print(result)


if __name__ == "__main__":
    main()
