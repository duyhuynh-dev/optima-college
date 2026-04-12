//! Criterion baseline for `run_optimize` (full pipeline: candidates → score → conflict filter).
//!
//! Uses `python-ml/output/sections_1269.csv` + `meetings_1269.csv` when present; otherwise
//! `benches/fixtures/*_99.csv` so CI and fresh clones can run `cargo bench`.

use std::path::PathBuf;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rust_kernel::optimize::{run_optimize, OptimizeParams, ScoreWeights};

fn dataset_paths() -> (PathBuf, PathBuf) {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out_s = manifest.join("../python-ml/output/sections_1269.csv");
    let out_m = manifest.join("../python-ml/output/meetings_1269.csv");
    if out_s.is_file() && out_m.is_file() {
        return (out_s, out_m);
    }
    (
        manifest.join("benches/fixtures/sections_99.csv"),
        manifest.join("benches/fixtures/meetings_99.csv"),
    )
}

fn bench_params() -> OptimizeParams {
    OptimizeParams {
        k: 4,
        max_results: 10,
        max_per_subject: 1,
        earliest_start_minutes: 0,
        subject_whitelist: vec![],
        subject_blacklist: vec![],
        weights: ScoreWeights {
            weekly: 0.35,
            evening: 0.20,
            early: 0.15,
            back_to_back: 0.15,
            busy_day: 0.15,
        },
        pareto: false,
        pareto_mode: "strict".into(),
        pareto_epsilon: 0.05,
        max_candidates: 2000,
        min_total_credits: 0.0,
        max_total_credits: 0.0,
    }
}

fn optimize_end_to_end(c: &mut Criterion) {
    let (sections, meetings) = dataset_paths();
    let params = bench_params();
    c.bench_function("run_optimize", |b| {
        b.iter(|| {
            let r = run_optimize(
                black_box(&sections),
                black_box(&meetings),
                black_box(params.clone()),
            );
            black_box(r.expect("run_optimize"));
        });
    });
}

criterion_group!(benches, optimize_end_to_end);
criterion_main!(benches);
