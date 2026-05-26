use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize},
    },
};

use serde::{Deserialize, Serialize};

use super::comparability::{TuneComparabilityWarning, TuneCoverageMetrics};
use crate::{profiles, recorder::IntervalRecord};
#[derive(Serialize, Deserialize)]
pub struct TuneSummary {
    pub schema_version: u32,
    pub tree_pid: u32,
    pub profiles_path: PathBuf,
    pub runs: u32,
    pub epoch_seconds: u64,
    pub warmup_seconds: u64,
    pub restore_policy: String,
    pub best_profile: String,
    pub candidate_order: Vec<TuneIterationOrder>,
    pub profile_stats: Vec<TuneProfileStats>,
    pub ranking_confidence: RankingConfidence,
    pub ranking_notes: Vec<String>,
    #[serde(default)]
    pub comparability_warnings: Vec<TuneComparabilityWarning>,
    pub candidates: Vec<TuneCandidateSummary>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TuneIterationOrder {
    pub iteration: u32,
    pub profiles: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TuneProfileStats {
    pub profile: String,
    pub valid_runs: usize,
    pub invalid_runs: usize,
    /// Raw diagnostic totals are only compared across fixed-duration tune runs.
    /// The explicit `raw_score_total` suffix prevents these serialized fields
    /// from being confused with normalized/rate-based comparison metrics.
    #[serde(alias = "median_diagnostic_score_total")]
    pub median_diagnostic_raw_score_total: u64,
    #[serde(alias = "iqr_diagnostic_score_total")]
    pub iqr_diagnostic_raw_score_total: u64,
    #[serde(alias = "worst_diagnostic_score_total")]
    pub worst_diagnostic_raw_score_total: u64,
    pub median_over_5ms: u64,
    pub iqr_over_5ms: u64,
    pub median_frame_p99_us: u64,
    pub iqr_frame_p99_us: u64,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RankingConfidence {
    High,
    Medium,
    Low,
    Unstable,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TuneCandidateSummary {
    pub profile: String,
    pub iteration: u32,
    pub run_dir: PathBuf,
    pub applied_tasks: usize,
    pub warmup_seconds: u64,
    pub measure_seconds: u64,
    pub interval_count: usize,
    pub samples: u64,
    pub scored_samples: u64,
    #[serde(alias = "diagnostic_score_total")]
    pub diagnostic_raw_score_total: u64,
    pub over_1ms: u64,
    pub over_2ms: u64,
    pub over_5ms: u64,
    pub max_latency_ns: u64,
    pub frame_count: usize,
    pub frame_max_ms: f64,
    pub frame_p99_ms: f64,
    pub frame_over_16ms: u64,
    pub frame_over_33ms: u64,
    pub frame_over_50ms: u64,
    pub coverage: TuneCoverageMetrics,
    pub valid: bool,
}

pub struct TuneCommandInput {
    pub tree_pid: u32,
    pub profiles_path: PathBuf,
    pub epoch_seconds: u64,
    pub warmup_seconds: u64,
    pub runs: u32,
    pub keep_best: bool,
    pub baseline_profile: Option<String>,
    pub out_dir: Option<PathBuf>,
    pub mangohud_log: Option<PathBuf>,
    pub enforce: bool,
    pub hwmon: bool,
}

pub struct TuneControl {
    pub stop_refresh: Arc<AtomicBool>,
    pub applied_tasks: Arc<AtomicUsize>,
}

pub struct TuneProfileRefreshInput {
    pub tree_pid: u32,
    pub profile: profiles::Profile,
    pub cache: profiles::ProfileApplyCache,
    pub force_restore_overwrite: bool,
    pub refresh_ms: u64,
    pub control: TuneControl,
    pub policy: crate::daemon_policy::DaemonPolicy,
    pub persistent_effect: bool,
    pub enforce: bool,
}

pub struct TuneMeasureResult {
    pub applied_tasks: usize,
    pub run_dir: PathBuf,
    pub interval_records: Vec<IntervalRecord>,
    pub frame_events: Vec<crate::recorder::FrameEvent>,
    pub coverage: TuneCoverageMetrics,
}
