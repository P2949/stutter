//! Main crate config model re-exports.
//!
//! The canonical `MonitorConfig` and all sub-structs now live in
//! `stutter-config`. This module re-exports them to preserve the existing
//! `crate::config::model::*` import paths throughout the main crate.

pub use stutter_config::{
    AlertConfig, CpuPerfConfig, DEFAULT_DESKTOP_ALERT_TIMEOUT_MS,
    DEFAULT_LIVE_DIAGNOSIS_CLUSTER_WINDOW_MS, DEFAULT_MANGOHUD_ALIGNMENT_POLL_MS,
    DEFAULT_MANGOHUD_TAIL_IDLE_SLEEP_MS, DiagnosisConfig, DisplayPathConfig, DmaBufConfig,
    DrmFenceConfig, EbpfSizingConfig, FocusConfig, HwmonConfig, KmsTimingConfig, MangoHudConfig,
    MonitorConfig, OutputConfig, ProbeConfig, RecordingConfig, RecordingRetentionConfig,
    RemoteConfig, RuntimeSlicesConfig, SafetyConfig, StreamConfig, TargetConfig, TimingConfig,
    UiConfig, WatchConfig, WaylandPresentationConfig,
};
