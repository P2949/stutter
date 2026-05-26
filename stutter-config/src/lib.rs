#![forbid(unsafe_code)]

//! Canonical configuration model and validation for the `stutter` workspace.
//!
//! This crate owns the authoritative configuration types and pure validation
//! logic. Runtime checks that require OS probing, eBPF availability, or
//! kernel feature detection remain in the main `stutter` crate.
//!
//! This crate must remain independent from the main `stutter` application crate.

pub mod config_model;
pub mod effective;
pub mod error;
pub mod layer;
pub mod model;
pub mod monitor_layer;
pub mod resolve;
pub mod schema;
pub mod source;
pub mod types;
pub mod validation;

pub use config_model::{
    AlertConfig, CpuPerfConfig, DEFAULT_DESKTOP_ALERT_TIMEOUT_MS, DEFAULT_FOREGROUND_POLL_MS,
    DEFAULT_LIVE_DIAGNOSIS_CLUSTER_WINDOW_MS, DEFAULT_MANGOHUD_ALIGNMENT_POLL_MS,
    DEFAULT_MANGOHUD_TAIL_IDLE_SLEEP_MS, DiagnosisConfig, DisplayPathConfig, DmaBufConfig,
    DrmFenceConfig, EbpfSizingConfig, FocusConfig, HwmonConfig, KmsTimingConfig, MangoHudConfig,
    MonitorConfig, OutputConfig, ProbeConfig, RecordingConfig, RecordingRetentionConfig,
    RemoteConfig, RuntimeSlicesConfig, SafetyConfig, StreamConfig, TargetConfig, TimingConfig,
    UiConfig, WatchConfig, WaylandPresentationConfig,
};
pub use error::ConfigError;
pub use types::{
    CsvStreamTarget, FocusSource, ForegroundSource, TARGET_PIDS_MAX, WaylandPresentationSource,
};
pub use validation::{validate_static_config, validate_target_max_tasks};

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use stutter_core::paths::StutterPaths;

    use super::{
        MonitorConfig, config_model::DEFAULT_FOREGROUND_POLL_MS, error::ConfigError,
        layer::ConfigLayer, model::ConfigModel, resolve::resolve_layers,
    };

    fn test_paths(root: &str) -> StutterPaths {
        StutterPaths::new(
            format!("{root}/state"),
            format!("{root}/config"),
            format!("{root}/cache"),
            format!("{root}/runs"),
            format!("{root}/audit.jsonl"),
            format!("{root}/daemon-state.json"),
            format!("{root}/agent.sock"),
        )
    }

    #[test]
    fn config_crate_exposes_minimal_model_layer_and_resolver() {
        let defaults = ConfigModel::with_paths(test_paths("/default"));
        let override_layer = ConfigLayer::with_paths(test_paths("/override"));

        let resolved = match resolve_layers(defaults, [override_layer]) {
            Ok(resolved) => resolved,
            Err(err) => panic!("expected config layers to resolve, got {err}"),
        };

        let paths = match resolved.model.paths() {
            Some(paths) => paths,
            None => panic!("expected resolved paths"),
        };

        assert_eq!(paths.state_dir, PathBuf::from("/override/state"));
        assert_eq!(paths.config_dir, PathBuf::from("/override/config"));
        assert_eq!(paths.cache_dir, PathBuf::from("/override/cache"));
        assert_eq!(paths.runs_dir, PathBuf::from("/override/runs"));
        assert_eq!(paths.audit_log, PathBuf::from("/override/audit.jsonl"));
        assert_eq!(
            paths.daemon_state,
            PathBuf::from("/override/daemon-state.json")
        );
        assert_eq!(paths.agent_socket, PathBuf::from("/override/agent.sock"));

        let error = ConfigError::missing_required_field("paths.runs_dir");
        assert_eq!(
            error.to_string(),
            "missing required config field 'paths.runs_dir'"
        );
    }

    #[test]
    fn config_crate_exposes_monitor_config() {
        // Verify the MonitorConfig is accessible and has sensible defaults.
        let config = MonitorConfig::default();
        assert_eq!(config.timing.summary_period_ms, 1_000);
        assert_eq!(config.focus.foreground_poll_ms, DEFAULT_FOREGROUND_POLL_MS);
        assert!(!config.has_explicit_target());
        assert!(!config.focus.auto_focus);
    }
}
