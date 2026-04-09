"""
Minimal data-quality checks on WesMaps silver CSVs (Phase 1 / Checkpoint A).

Validates structure, required fields, duplicate keys, and optional drift vs last run.
Exit code 1 on failure (for CI / schedulers).

Example:
  PYTHONPATH=src python3 -m optima_ingest.dq_check --term 1269 --input-dir output
"""

from __future__ import annotations

import argparse
import csv
import json
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any


@dataclass(frozen=True)
class Counts:
    courses: int
    sections: int
    meetings: int

    def to_dict(self) -> dict[str, int]:
        return {
            "courses": self.courses,
            "sections": self.sections,
            "meetings": self.meetings,
        }


def _read_csv(path: Path) -> tuple[list[str], list[dict[str, str]]]:
    with path.open(newline="", encoding="utf-8") as f:
        r = csv.DictReader(f)
        rows = list(r)
        return list(r.fieldnames or []), rows


def _require_cols(fieldnames: list[str], required: set[str], name: str) -> None:
    missing = required - set(fieldnames)
    if missing:
        raise SystemExit(f"{name}: missing columns {sorted(missing)}; have {fieldnames}")


def _check_courses(fieldnames: list[str], rows: list[dict[str, str]]) -> None:
    _require_cols(fieldnames, {"term", "subject_code", "course_code", "course_title"}, "courses")
    if not rows:
        raise SystemExit("courses CSV has no data rows")
    seen: set[tuple[str, str, str]] = set()
    for i, row in enumerate(rows, start=2):
        t, s, c = row.get("term", "").strip(), row.get("subject_code", "").strip(), row.get(
            "course_code", ""
        ).strip()
        if not t or not s or not c:
            raise SystemExit(f"courses row {i}: empty term/subject_code/course_code")
        key = (t, s, c)
        if key in seen:
            raise SystemExit(f"courses row {i}: duplicate (term, subject_code, course_code)={key}")
        seen.add(key)


def _check_sections(fieldnames: list[str], rows: list[dict[str, str]]) -> None:
    if not rows:
        return
    _require_cols(fieldnames, {"term", "subject_code", "course_code", "section"}, "sections")
    seen: set[tuple[str, str, str, str]] = set()
    for i, row in enumerate(rows, start=2):
        t = row.get("term", "").strip()
        s = row.get("subject_code", "").strip()
        cc = row.get("course_code", "").strip()
        sec = row.get("section", "").strip()
        if not t or not s or not cc or not sec:
            raise SystemExit(f"sections row {i}: empty term/subject_code/course_code/section")
        key = (t, s, cc, sec)
        if key in seen:
            raise SystemExit(f"sections row {i}: duplicate section key={key}")
        seen.add(key)


def _check_meetings(fieldnames: list[str], rows: list[dict[str, str]]) -> None:
    if not rows:
        return
    _require_cols(
        fieldnames,
        {"term", "subject_code", "course_code", "section", "day_code", "start_time", "end_time"},
        "meetings",
    )
    seen: set[tuple[str, str, str, str, str, str, str]] = set()
    for i, row in enumerate(rows, start=2):
        t = row.get("term", "").strip()
        s = row.get("subject_code", "").strip()
        cc = row.get("course_code", "").strip()
        sec = row.get("section", "").strip()
        d = row.get("day_code", "").strip()
        st = row.get("start_time", "").strip()
        en = row.get("end_time", "").strip()
        if not all((t, s, cc, sec, d, st, en)):
            raise SystemExit(f"meetings row {i}: empty required meeting field")
        key = (t, s, cc, sec, d, st, en)
        if key in seen:
            raise SystemExit(f"meetings row {i}: duplicate meeting key")
        seen.add(key)


def run_dq(term: str, input_dir: Path) -> Counts:
    courses_p = input_dir / f"courses_{term}.csv"
    sections_p = input_dir / f"sections_{term}.csv"
    meetings_p = input_dir / f"meetings_{term}.csv"
    for p in (courses_p, sections_p, meetings_p):
        if not p.exists():
            raise SystemExit(f"Missing {p}")

    fn_c, courses = _read_csv(courses_p)
    fn_s, sections = _read_csv(sections_p)
    fn_m, meetings = _read_csv(meetings_p)

    _check_courses(fn_c, courses)
    _check_sections(fn_s, sections)
    _check_meetings(fn_m, meetings)

    return Counts(
        courses=len(courses),
        sections=len(sections),
        meetings=len(meetings),
    )


def _drift_check(
    counts: Counts,
    baseline_path: Path,
    drift_max_drop_ratio: float,
) -> None:
    if not baseline_path.exists():
        print(
            f"No baseline at {baseline_path}; next run will compare after this write.",
            file=sys.stderr,
        )
        return
    raw: dict[str, Any] = json.loads(baseline_path.read_text(encoding="utf-8"))
    prev = Counts(
        courses=int(raw["courses"]),
        sections=int(raw["sections"]),
        meetings=int(raw["meetings"]),
    )
    for name, cur, old in (
        ("courses", counts.courses, prev.courses),
        ("sections", counts.sections, prev.sections),
        ("meetings", counts.meetings, prev.meetings),
    ):
        if old <= 0:
            continue
        drop = (old - cur) / old
        if drop > drift_max_drop_ratio:
            raise SystemExit(
                f"Drift: {name} dropped {drop:.1%} (was {old}, now {cur}); "
                f"max allowed drop ratio {drift_max_drop_ratio:.1%}",
            )


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(description="DQ checks on WesMaps silver CSVs.")
    p.add_argument("--term", required=True, help="Term code, e.g. 1269")
    p.add_argument(
        "--input-dir",
        type=Path,
        default=Path("output"),
        help="Directory with courses_<term>.csv etc.",
    )
    p.add_argument(
        "--baseline",
        type=Path,
        default=None,
        help="JSON file with prior row counts (default: <input-dir>/.dq_baseline_<term>.json)",
    )
    p.add_argument(
        "--drift-max-drop-ratio",
        type=float,
        default=None,
        help="If set, compare to baseline and fail when any count drops by more than this fraction (0–1).",
    )
    return p


def main() -> None:
    args = build_parser().parse_args()
    baseline = args.baseline
    if baseline is None:
        baseline = args.input_dir / f".dq_baseline_{args.term}.json"

    counts = run_dq(args.term, args.input_dir)

    if args.drift_max_drop_ratio is not None:
        if not (0.0 <= args.drift_max_drop_ratio <= 1.0):
            raise SystemExit("--drift-max-drop-ratio must be between 0 and 1")
        _drift_check(counts, baseline, args.drift_max_drop_ratio)
        baseline.parent.mkdir(parents=True, exist_ok=True)
        baseline.write_text(json.dumps(counts.to_dict(), indent=2) + "\n", encoding="utf-8")

    print(
        json.dumps(
            {"ok": True, "term": args.term, "counts": counts.to_dict()},
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
