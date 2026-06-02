use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::tune::{
    RankingConfidence, recommendation::TuneRecommendationVerdict,
    statistics::FormalMetricComparison,
};

#[derive(Debug, Clone)]
pub struct RecommendCommandInput {
    pub baseline: Vec<PathBuf>,
    pub tune: PathBuf,
    pub json: bool,
    pub markdown: Option<PathBuf>,
    pub html: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaselineTuneRecommendation {
    pub schema_version: u32,
    pub baseline_run: PathBuf,
    #[serde(default)]
    pub baseline_runs: Vec<PathBuf>,
    pub tune_dir: PathBuf,
    pub best_profile: Option<String>,
    pub confidence: RankingConfidence,
    #[serde(default)]
    pub confidence_metadata: Option<BaselineTuneConfidenceMetadata>,
    pub verdict: TuneRecommendationVerdict,
    pub summary: String,
    #[serde(default)]
    pub baseline_valid_runs: usize,
    #[serde(default)]
    pub baseline_invalid_runs: usize,
    /// Median diagnostic score across valid baseline runs. For schema v2 files
    /// this was the single baseline run score; schema v3 generalizes it to
    /// repeated baselines while preserving the field name for compatibility.
    pub diagnostic_baseline_raw_score_total: u64,
    #[serde(alias = "best_median_diagnostic_score_total")]
    pub best_median_diagnostic_raw_score_total: Option<u64>,
    pub score_delta_abs: Option<i64>,
    pub score_delta_percent: Option<f64>,
    #[serde(default)]
    pub score_effect_size: Option<f64>,
    #[serde(default)]
    pub score_noise_ratio: Option<f64>,

    pub baseline_over_5ms: u64,
    pub best_median_over_5ms: Option<u64>,
    pub over_5ms_delta_abs: Option<i64>,
    #[serde(default)]
    pub over_5ms_effect_size: Option<f64>,
    #[serde(default)]
    pub over_5ms_noise_ratio: Option<f64>,

    pub baseline_frame_p99_ms: Option<f64>,
    pub best_median_frame_p99_ms: Option<f64>,
    pub frame_p99_delta_ms: Option<f64>,
    #[serde(default)]
    pub frame_p99_effect_size: Option<f64>,
    #[serde(default)]
    pub frame_p99_noise_ratio: Option<f64>,

    #[serde(default)]
    pub formal_metrics: Vec<FormalMetricComparison>,

    pub warnings: Vec<String>,
    pub next_steps: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaselineTuneConfidenceMetadata {
    pub ranking_confidence: RankingConfidence,
    pub ranking_notes: Vec<String>,
    pub tune_runs: u32,
    pub tune_epoch_seconds: u64,
    pub tune_warmup_seconds: u64,
    pub best_profile_valid_runs: Option<usize>,
    pub best_profile_invalid_runs: Option<usize>,
    pub baseline_valid_runs: usize,
    pub baseline_invalid_runs: usize,
}
