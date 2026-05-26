//! Main crate config model re-exports.
//!
//! The canonical `MonitorConfig` and all sub-structs now live in
//! `stutter-config`. This module re-exports the compatibility surface still used by
//! the main crate. Wider public API re-exports should go through `crate::api`.

#[cfg(test)]
pub(crate) use stutter_config::DEFAULT_LIVE_DIAGNOSIS_CLUSTER_WINDOW_MS;
pub use stutter_config::{
    AlertConfig, CpuPerfConfig, EbpfSizingConfig, FocusConfig, HwmonConfig, MangoHudConfig,
    MonitorConfig, OutputConfig, ProbeConfig, RecordingConfig, RecordingRetentionConfig,
    RemoteConfig, RuntimeSlicesConfig, SafetyConfig, StreamConfig, TargetConfig, TimingConfig,
    UiConfig, WatchConfig,
};
