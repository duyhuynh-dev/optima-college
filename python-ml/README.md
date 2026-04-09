# python-ml

This workspace is for:

- deterministic data ingestion from public course sources
- feature engineering experiments
- model training and evaluation

## Setup

```bash
python3 -m venv .venv
source .venv/bin/activate
pip install -U pip
pip install -e .[dev]
```

## Run WesMaps ingestion

```bash
python -m optima_ingest.cli --term 1269 --out-dir output
```

Expected files:

- `output/courses_1269.csv`
- `output/sections_1269.csv`
- `output/meetings_1269.csv`

## Notes

- Parser now ingests `crse_list` pages and extracts section-level fields:
  - section code (for example `01`)
  - instructor
  - meeting pattern
  - normalized meeting rows (`day_code`, `start_time`, `end_time`)
  - course reference id (`crse`)
  - seasonal term label (`Fall 2026`, `Spring 2027`)
- If dependencies are missing, run `pip install -e .[dev]` first.
