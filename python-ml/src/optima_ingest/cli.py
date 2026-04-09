from __future__ import annotations

import argparse
from pathlib import Path

from .wesmaps import run_ingestion


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Ingest Wesleyan WesMaps public catalog data.")
    parser.add_argument("--term", required=True, help="Term code, e.g. 1269")
    parser.add_argument(
        "--out-dir",
        default="output",
        help="Output directory for normalized CSV files (default: output)",
    )
    parser.add_argument(
        "--no-bronze",
        action="store_true",
        help="Do not write raw HTML snapshots under <out-dir>/bronze/<term>/ (saves disk).",
    )
    return parser


def main() -> None:
    parser = build_parser()
    args = parser.parse_args()
    run_ingestion(term=args.term, out_dir=Path(args.out_dir), save_bronze=not args.no_bronze)


if __name__ == "__main__":
    main()
