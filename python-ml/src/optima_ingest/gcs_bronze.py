"""
Upload local bronze HTML snapshots to a GCS bucket (optional Phase 1.1).

Requires: pip install 'google-cloud-storage' (or optima-python-ml[gcs]).

Example:
  export GCP_PROJECT=optima-college
  PYTHONPATH=src python3 -m optima_ingest.gcs_bronze \\
    --term 1269 --out-dir output --bucket optima-college-wesmaps-bronze --prefix optima/bronze
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path


def main() -> None:
    try:
        from google.cloud import storage
    except ImportError as e:
        raise SystemExit(
            "google-cloud-storage is required. Install: pip install google-cloud-storage",
        ) from e

    p = argparse.ArgumentParser(description="Sync local bronze HTML to GCS.")
    p.add_argument("--term", required=True, help="Term code, e.g. 1269")
    p.add_argument(
        "--out-dir",
        type=Path,
        default=Path("output"),
        help="Output dir containing bronze/<term>/",
    )
    p.add_argument(
        "--bucket",
        required=True,
        help="GCS bucket name (no gs:// prefix)",
    )
    p.add_argument(
        "--prefix",
        default="bronze",
        help="Object prefix inside the bucket (no leading slash)",
    )
    args = p.parse_args()

    project = os.environ.get("GCP_PROJECT") or os.environ.get("GOOGLE_CLOUD_PROJECT") or ""
    if not project:
        print("Warning: set GCP_PROJECT for consistent client defaults.", file=sys.stderr)

    local_root = args.out_dir / "bronze" / args.term
    if not local_root.is_dir():
        raise SystemExit(f"Bronze directory not found: {local_root} (run ingest without --no-bronze)")

    client = storage.Client(project=project or None)
    bucket = client.bucket(args.bucket)
    prefix = args.prefix.strip("/").rstrip("/")
    uploaded = 0
    for path in sorted(local_root.rglob("*")):
        if not path.is_file():
            continue
        rel = path.relative_to(local_root)
        blob_name = f"{prefix}/{args.term}/{rel.as_posix()}"
        blob = bucket.blob(blob_name)
        blob.upload_from_filename(str(path))
        uploaded += 1

    print(
        json.dumps(
            {"ok": True, "bucket": args.bucket, "uploaded_files": uploaded},
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
