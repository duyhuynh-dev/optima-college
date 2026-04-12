use std::path::{Path, PathBuf};

use tonic::{Request, Response, Status};

use crate::conflicts::{detect_conflicts, load_meetings_from_csv, ConflictPair};
use crate::optimize::{self, OptimizeParams, ScoreWeights as OptWeights};

pub mod optima {
    pub mod v1 {
        tonic::include_proto!("optima.v1");
    }
}

use optima::v1::kernel_server::Kernel;
use optima::v1::{
    CheckConflictsRequest, CheckConflictsResponse, ConflictPair as ProtoConflict, Empty,
    HealthResponse, OptimizeRequest, OptimizeResponse, ScheduleOption as ProtoScheduleOption,
    ScoreWeights as ProtoScoreWeights,
};

#[derive(Clone)]
pub struct KernelService {
    pub default_csv: PathBuf,
    pub default_sections_csv: PathBuf,
}

fn map_pair(c: ConflictPair) -> ProtoConflict {
    ProtoConflict {
        day_code: c.day_code,
        day_name: c.day_name,
        left_section: c.left_section,
        right_section: c.right_section,
        left_start: c.left_start,
        left_end: c.left_end,
        right_start: c.right_start,
        right_end: c.right_end,
    }
}

#[tonic::async_trait]
impl Kernel for KernelService {
    async fn check_conflicts(
        &self,
        request: Request<CheckConflictsRequest>,
    ) -> Result<Response<CheckConflictsResponse>, Status> {
        let req = request.into_inner();
        if req.sections.is_empty() {
            return Err(Status::invalid_argument(
                "sections must contain at least one section id",
            ));
        }
        let path = if req.csv_path.is_empty() {
            self.default_csv.clone()
        } else {
            PathBuf::from(req.csv_path)
        };
        if !Path::new(&path).exists() {
            return Err(Status::not_found(format!(
                "meetings csv not found: {}",
                path.display()
            )));
        }
        let meetings = load_meetings_from_csv(&path).map_err(Status::internal)?;
        let sections: Vec<String> = req.sections.into_iter().collect();
        let conflicts = detect_conflicts(&meetings, &sections);
        let conflicts_proto: Vec<ProtoConflict> = conflicts.into_iter().map(map_pair).collect();
        let has_conflict = !conflicts_proto.is_empty();
        let n = conflicts_proto.len() as u32;
        Ok(Response::new(CheckConflictsResponse {
            status: "ok".into(),
            has_conflict,
            conflict_count: n,
            conflicts: conflicts_proto,
        }))
    }

    async fn optimize(
        &self,
        request: Request<OptimizeRequest>,
    ) -> Result<Response<OptimizeResponse>, Status> {
        let req = request.into_inner();
        let sections_path = if req.sections_csv_path.is_empty() {
            self.default_sections_csv.clone()
        } else {
            PathBuf::from(req.sections_csv_path)
        };
        let meetings_path = if req.meetings_csv_path.is_empty() {
            self.default_csv.clone()
        } else {
            PathBuf::from(req.meetings_csv_path)
        };
        if !Path::new(&sections_path).exists() {
            return Err(Status::not_found(format!(
                "sections csv not found: {}",
                sections_path.display()
            )));
        }
        if !Path::new(&meetings_path).exists() {
            return Err(Status::not_found(format!(
                "meetings csv not found: {}",
                meetings_path.display()
            )));
        }

        let base_weights = match req.weights {
            Some(ref w) => OptWeights {
                weekly: w.w_weekly,
                evening: w.w_evening,
                early: w.w_early,
                back_to_back: w.w_back_to_back,
                busy_day: w.w_busy_day,
            },
            None => OptWeights {
                weekly: 0.0,
                evening: 0.0,
                early: 0.0,
                back_to_back: 0.0,
                busy_day: 0.0,
            },
        };

        let params = OptimizeParams {
            k: req.k,
            max_results: req.max_results,
            max_per_subject: req.max_per_subject,
            earliest_start_minutes: req.earliest_start_minutes,
            subject_whitelist: req.subject_whitelist,
            subject_blacklist: req.subject_blacklist,
            weights: base_weights,
            pareto: req.pareto,
            pareto_mode: if req.pareto_mode.is_empty() {
                "strict".into()
            } else {
                req.pareto_mode
            },
            pareto_epsilon: req.pareto_epsilon,
            max_candidates: req.max_candidates,
            min_total_credits: req.min_total_credits,
            max_total_credits: req.max_total_credits,
        };

        let (options, effective, reason) =
            optimize::run_optimize(&sections_path, &meetings_path, params).map_err(|e| {
                Status::internal(e)
            })?;

        let proto_opts: Vec<ProtoScheduleOption> = options
            .into_iter()
            .map(|o| ProtoScheduleOption {
                id: o.id,
                sections: o.sections,
                expected_utility: o.expected_utility,
                stress_score: o.stress_score,
                academic_load_score: o.academic_load_score,
                lifestyle_penalty_score: o.lifestyle_penalty_score,
            })
            .collect();

        let eff = ProtoScoreWeights {
            w_weekly: effective.weekly,
            w_evening: effective.evening,
            w_early: effective.early,
            w_back_to_back: effective.back_to_back,
            w_busy_day: effective.busy_day,
        };

        Ok(Response::new(OptimizeResponse {
            status: "ok".into(),
            reason: reason.unwrap_or_default(),
            options: proto_opts,
            effective_weights: Some(eff),
        }))
    }

    async fn health(&self, _request: Request<Empty>) -> Result<Response<HealthResponse>, Status> {
        Ok(Response::new(HealthResponse {
            status: "ok".into(),
            service: "rust-kernel".into(),
        }))
    }
}
