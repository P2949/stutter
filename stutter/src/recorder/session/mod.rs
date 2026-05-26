use std::{
    path::PathBuf,
    time::{Instant, SystemTime},
};

use serde::{Deserialize, Serialize};

use super::event_types::FrameEvent;
use crate::{config::model::MonitorConfig, foreground::ForegroundEvent};

mod finalize;
mod metadata;
mod prepare;
mod warnings;
mod writers;

#[derive(Debug)]
pub struct RecordingRun {
    pub run_name: Option<String>,
    pub run_dir: PathBuf,
    pub started_at: SystemTime,
    pub started_instant: Instant,
    pub monotonic_start_ns: Option<u64>,
    pub mangohud_start_offset: Option<u64>,
    pub mangohud_first_frame_monotonic_ns: Option<u64>,
    pub mangohud_first_frame_raw_elapsed_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CpuPerfStatus {
    pub sample_count: u64,
    pub active_counter_tasks: u64,
    pub skipped_counter_tasks: u64,
    pub open_errors: u64,
    pub read_errors: u64,
    pub last_error: Option<String>,
}

pub struct FinalizeRecordingInput<'a> {
    pub recorder: &'a super::LiveRecorder,
    pub config: &'a MonitorConfig,
    pub tree_pids: &'a [u32],
    pub stop_reason: &'a str,
    pub tasks: &'a crate::tasks::TaskTracker,
    pub frame_events: &'a [FrameEvent],
    pub block_io_correlation_basis: String,
    pub block_io_correlation_confidence: String,
    pub native_cgroup_filter: crate::ebpf_loader::NativeCgroupFilterStatus,
    pub probe_activation_warnings: Vec<super::session_files::RecordedProbeActivationWarning>,
    pub drop_counters: crate::ebpf_loader::DropCountersSnapshot,
    pub cpu_perf_status: Option<CpuPerfStatus>,
    pub focus_mode: Option<String>,
    pub final_focus_kind: Option<String>,
    pub focus_switch_count: u64,
    pub current_focus: Option<crate::focus::ResolvedFocus>,
    pub final_foreground_event: Option<ForegroundEvent>,
}

#[cfg(test)]
use std::{fs, path::Path};

pub use finalize::finalize_recording;
#[cfg(test)]
use metadata::recorded_spike;
#[cfg(test)]
pub(crate) use metadata::saturating_u128_to_u64;
pub(crate) use metadata::{elapsed_ms_from_monotonic, monotonic_now_ns};
pub use metadata::{recorded_config, recorded_time};
pub use prepare::prepare_recording;
pub use warnings::{
    RecordingWarning, RecordingWarningKind, print_recording_warnings, recording_warnings,
};
#[cfg(test)]
use writers::write_json;

#[cfg(test)]
use crate::{
    artifacts::ArtifactKind,
    recorder::{LiveRecorder, MetadataFile, SessionFile, SyncTracker},
};

#[cfg(test)]
mod tests;
