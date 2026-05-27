use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::tune::{RankingConfidence, recommendation::TuneRecommendationVerdict};

#[derive(Debug, Clone)]
pub struct RecommendCommandInput {
    pub baseline: PathBuf,
    pub tune: PathBuf,
    pub json: bool,
    pub markdown: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaselineTuneRecommendation {
    pub schema_version: u32,
    pub baseline_run: PathBuf,
    pub tune_dir: PathBuf,
    pub best_profile: Option<String>,
    pub confidence: RankingConfidence,
    pub verdict: TuneRecommendationVerdict,
    pub summary: String,
    pub diagnostic_baseline_raw_score_total: u64,
    #[serde(alias = "best_median_diagnostic_score_total")]
    pub best_median_diagnostic_raw_score_total: Option<u64>,
    pub score_delta_abs: Option<i64>,
    pub score_delta_percent: Option<f64>,

    pub baseline_over_5ms: u64,
    pub best_median_over_5ms: Option<u64>,
    pub over_5ms_delta_abs: Option<i64>,

    pub baseline_frame_p99_ms: Option<f64>,
    pub best_median_frame_p99_ms: Option<f64>,
    pub frame_p99_delta_ms: Option<f64>,

    pub warnings: Vec<String>,
    pub next_steps: Vec<String>,
}
