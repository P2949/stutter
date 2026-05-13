use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::{
    actions::{RollbackToken, SafetyClass},
    daemon::policy::DaemonMode,
};

pub const DAEMON_STATE_SCHEMA_VERSION: u32 = 1;

pub fn default_daemon_state_snapshot_path() -> PathBuf {
    let mut path = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    path.push(".local");
    path.push("state");
    path.push("stutter");
    path.push("autotune");
    path.push("daemon_state.json");
    path
}

pub fn load_daemon_state(path: &Path) -> anyhow::Result<DaemonState> {
    let file = fs::File::open(path)
        .with_context(|| format!("failed to open daemon state snapshot {}", path.display()))?;

    let state: DaemonState = serde_json::from_reader(file)
        .with_context(|| format!("failed to parse daemon state snapshot {}", path.display()))?;

    if state.schema_version != DAEMON_STATE_SCHEMA_VERSION {
        anyhow::bail!(
            "unsupported daemon state snapshot schema_version={} in {}",
            state.schema_version,
            path.display()
        );
    }

    Ok(state)
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DaemonPhase {
    Disabled,
    Init,
    Recover,
    #[serde(rename = "observing", alias = "observe")]
    Observe,
    #[serde(rename = "planning", alias = "decide")]
    Decide,
    #[serde(rename = "applying", alias = "apply")]
    Apply,
    #[serde(rename = "measuring", alias = "measure")]
    Measure,
    #[serde(rename = "keeping", alias = "keep")]
    Keep,
    #[serde(rename = "reverting", alias = "rollback")]
    Rollback,
    Cooldown,
    Faulted,
    Shutdown,
}

impl DaemonPhase {
    pub fn lifecycle_label(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Init => "init",
            Self::Recover => "recover",
            Self::Observe => "observe",
            Self::Decide => "decide",
            Self::Apply => "apply",
            Self::Measure => "measure",
            Self::Keep => "keep",
            Self::Rollback => "rollback",
            Self::Cooldown => "cooldown",
            Self::Faulted => "faulted",
            Self::Shutdown => "shutdown",
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Disabled | Self::Faulted | Self::Shutdown)
    }

    pub fn is_faulted(self) -> bool {
        matches!(self, Self::Faulted)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonState {
    pub schema_version: u32,
    pub mode: DaemonMode,
    pub phase: DaemonPhase,
    #[serde(default)]
    pub cooldown_until_unix_nanos: Option<u128>,
    pub active_target: Option<DaemonTargetState>,
    pub active_experiment: Option<DaemonExperimentState>,
    pub active_rollback: Option<DaemonRollbackState>,
    pub last_decision: Option<DaemonDecisionState>,
    pub degraded: Vec<DaemonDegradedStatus>,
    pub faulted: Option<DaemonFaultState>,
}

impl Default for DaemonState {
    fn default() -> Self {
        Self {
            schema_version: DAEMON_STATE_SCHEMA_VERSION,
            mode: DaemonMode::Observe,
            phase: DaemonPhase::Disabled,
            cooldown_until_unix_nanos: None,
            active_target: None,
            active_experiment: None,
            active_rollback: None,
            last_decision: None,
            degraded: Vec::new(),
            faulted: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonTargetState {
    pub root_pid: Option<u32>,
    pub active_targets: usize,
    pub comm: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonExperimentState {
    pub experiment_id: String,
    pub action_id: String,
    pub candidate_name: Option<String>,
    pub safety_class: SafetyClass,
    pub started_unix_nanos: Option<u128>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonRollbackState {
    pub action_id: String,
    pub rollback_available: bool,
    pub token: Option<RollbackToken>,
    pub manual_restore_command: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonDecisionState {
    pub decision: String,
    pub reason: String,
    pub unix_nanos: Option<u128>,
    pub score_total: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonDegradedStatus {
    pub category: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonFaultState {
    pub reason: String,
    pub manual_restore_command: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DaemonStateSnapshotWriter {
    path: PathBuf,
}

impl DaemonStateSnapshotWriter {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }

    pub fn default_path() -> PathBuf {
        default_daemon_state_snapshot_path()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn write(&self, state: &DaemonState) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create daemon state snapshot directory {}",
                    parent.display()
                )
            })?;
        }

        let temporary_path = temporary_daemon_state_snapshot_path(&self.path);
        {
            let mut file = fs::File::create(&temporary_path).with_context(|| {
                format!(
                    "failed to create daemon state snapshot temp file {}",
                    temporary_path.display()
                )
            })?;

            serde_json::to_writer_pretty(&mut file, state).with_context(|| {
                format!(
                    "failed to serialize daemon state snapshot {}",
                    temporary_path.display()
                )
            })?;
            file.write_all(b"\n").with_context(|| {
                format!(
                    "failed to terminate daemon state snapshot {}",
                    temporary_path.display()
                )
            })?;
            file.sync_all().with_context(|| {
                format!(
                    "failed to sync daemon state snapshot temp file {}",
                    temporary_path.display()
                )
            })?;
        }

        fs::rename(&temporary_path, &self.path).with_context(|| {
            format!(
                "failed to atomically replace daemon state snapshot {} with {}",
                self.path.display(),
                temporary_path.display()
            )
        })?;

        Ok(())
    }
}

fn temporary_daemon_state_snapshot_path(path: &Path) -> PathBuf {
    let mut temporary_path = path.to_path_buf();
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("daemon_state.json");
    temporary_path.set_file_name(format!("{file_name}.tmp"));
    temporary_path
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-daemon-state-test-{name}-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn daemon_state_default_serializes_with_schema_version() {
        let state = DaemonState::default();

        let json = serde_json::to_string(&state).unwrap();

        assert!(json.contains("\"schema_version\":1"));
        assert!(json.contains("\"mode\":\"observe\""));
        assert!(json.contains("\"phase\":\"disabled\""));
    }

    #[test]
    fn daemon_phase_helpers_report_lifecycle_labels_and_terminal_states() {
        assert_eq!(DaemonPhase::Init.lifecycle_label(), "init");
        assert_eq!(DaemonPhase::Recover.lifecycle_label(), "recover");
        assert_eq!(DaemonPhase::Observe.lifecycle_label(), "observe");
        assert_eq!(DaemonPhase::Decide.lifecycle_label(), "decide");
        assert_eq!(DaemonPhase::Apply.lifecycle_label(), "apply");
        assert_eq!(DaemonPhase::Measure.lifecycle_label(), "measure");
        assert_eq!(DaemonPhase::Rollback.lifecycle_label(), "rollback");
        assert_eq!(DaemonPhase::Cooldown.lifecycle_label(), "cooldown");
        assert_eq!(DaemonPhase::Faulted.lifecycle_label(), "faulted");
        assert_eq!(DaemonPhase::Shutdown.lifecycle_label(), "shutdown");

        assert!(DaemonPhase::Disabled.is_terminal());
        assert!(DaemonPhase::Faulted.is_terminal());
        assert!(DaemonPhase::Shutdown.is_terminal());
        assert!(!DaemonPhase::Observe.is_terminal());
        assert!(!DaemonPhase::Measure.is_terminal());

        assert!(DaemonPhase::Faulted.is_faulted());
        assert!(!DaemonPhase::Shutdown.is_faulted());
    }

    #[test]
    fn daemon_phase_preserves_existing_serialized_names_and_accepts_new_aliases() {
        let serialized_names = [
            (DaemonPhase::Disabled, "\"disabled\""),
            (DaemonPhase::Init, "\"init\""),
            (DaemonPhase::Recover, "\"recover\""),
            (DaemonPhase::Observe, "\"observing\""),
            (DaemonPhase::Decide, "\"planning\""),
            (DaemonPhase::Apply, "\"applying\""),
            (DaemonPhase::Measure, "\"measuring\""),
            (DaemonPhase::Keep, "\"keeping\""),
            (DaemonPhase::Rollback, "\"reverting\""),
            (DaemonPhase::Cooldown, "\"cooldown\""),
            (DaemonPhase::Faulted, "\"faulted\""),
            (DaemonPhase::Shutdown, "\"shutdown\""),
        ];

        for (phase, expected_json) in serialized_names {
            assert_eq!(serde_json::to_string(&phase).unwrap(), expected_json);
        }

        let accepted_names = [
            ("\"disabled\"", DaemonPhase::Disabled),
            ("\"init\"", DaemonPhase::Init),
            ("\"recover\"", DaemonPhase::Recover),
            ("\"observing\"", DaemonPhase::Observe),
            ("\"observe\"", DaemonPhase::Observe),
            ("\"planning\"", DaemonPhase::Decide),
            ("\"decide\"", DaemonPhase::Decide),
            ("\"applying\"", DaemonPhase::Apply),
            ("\"apply\"", DaemonPhase::Apply),
            ("\"measuring\"", DaemonPhase::Measure),
            ("\"measure\"", DaemonPhase::Measure),
            ("\"keeping\"", DaemonPhase::Keep),
            ("\"keep\"", DaemonPhase::Keep),
            ("\"reverting\"", DaemonPhase::Rollback),
            ("\"rollback\"", DaemonPhase::Rollback),
            ("\"cooldown\"", DaemonPhase::Cooldown),
            ("\"faulted\"", DaemonPhase::Faulted),
            ("\"shutdown\"", DaemonPhase::Shutdown),
        ];

        for (json, expected_phase) in accepted_names {
            assert_eq!(
                serde_json::from_str::<DaemonPhase>(json).unwrap(),
                expected_phase
            );
        }
    }

    #[test]
    fn daemon_state_can_store_live_runtime_fields() {
        let state = DaemonState {
            mode: DaemonMode::ApplyLowRisk,
            phase: DaemonPhase::Measure,
            active_target: Some(DaemonTargetState {
                root_pid: Some(1234),
                active_targets: 12,
                comm: Some("game".to_owned()),
            }),
            active_experiment: Some(DaemonExperimentState {
                experiment_id: "experiment-1".to_owned(),
                action_id: "cpu-affinity-profile:game".to_owned(),
                candidate_name: Some("game".to_owned()),
                safety_class: SafetyClass::ReversibleLowRisk,
                started_unix_nanos: Some(100),
            }),
            active_rollback: Some(DaemonRollbackState {
                action_id: "cpu-affinity-profile:game".to_owned(),
                rollback_available: true,
                token: None,
                manual_restore_command: Some("stutter autotune restore".to_owned()),
            }),
            last_decision: Some(DaemonDecisionState {
                decision: "candidate_applied".to_owned(),
                reason: "candidate passed gates".to_owned(),
                unix_nanos: Some(200),
                score_total: Some(300),
            }),
            degraded: vec![DaemonDegradedStatus {
                category: "data_quality".to_owned(),
                message: "low scored samples".to_owned(),
            }],
            faulted: None,
            ..DaemonState::default()
        };

        let decoded: DaemonState =
            serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();

        assert_eq!(decoded.mode, DaemonMode::ApplyLowRisk);
        assert_eq!(decoded.phase, DaemonPhase::Measure);
        assert_eq!(
            decoded
                .active_target
                .as_ref()
                .and_then(|target| target.root_pid),
            Some(1234)
        );
        assert!(decoded.active_rollback.unwrap().rollback_available);
        assert_eq!(decoded.degraded.len(), 1);
    }

    #[test]
    fn daemon_state_snapshot_writer_atomically_writes_json_and_removes_temp_file() {
        let dir = temp_dir("snapshot-writer");
        let path = dir.join("daemon_state.json");
        let writer = DaemonStateSnapshotWriter::new(&path);
        let state = DaemonState {
            mode: DaemonMode::ApplyLowRisk,
            phase: DaemonPhase::Cooldown,
            cooldown_until_unix_nanos: Some(9_000),
            degraded: vec![DaemonDegradedStatus {
                category: "data_quality".to_owned(),
                message: "low data quality".to_owned(),
            }],
            ..DaemonState::default()
        };

        writer.write(&state).unwrap();

        let decoded = load_daemon_state(&path).unwrap();

        assert_eq!(writer.path(), path.as_path());
        assert_eq!(decoded.mode, DaemonMode::ApplyLowRisk);
        assert_eq!(decoded.phase, DaemonPhase::Cooldown);
        assert_eq!(decoded.cooldown_until_unix_nanos, Some(9_000));
        assert_eq!(decoded.degraded[0].category, "data_quality");
        assert!(!temporary_daemon_state_snapshot_path(&path).exists());

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn load_daemon_state_rejects_unsupported_schema_version() {
        let dir = temp_dir("unsupported-schema");
        let path = dir.join("daemon_state.json");
        let state = DaemonState {
            schema_version: DAEMON_STATE_SCHEMA_VERSION + 1,
            ..DaemonState::default()
        };

        serde_json::to_writer_pretty(fs::File::create(&path).unwrap(), &state).unwrap();

        let err = load_daemon_state(&path).unwrap_err();

        assert!(
            err.to_string()
                .contains("unsupported daemon state snapshot schema_version")
        );

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn default_daemon_state_snapshot_path_matches_autotune_state_directory() {
        let path = default_daemon_state_snapshot_path();
        let rendered = path.to_string_lossy();

        assert!(rendered.ends_with(".local/state/stutter/autotune/daemon_state.json"));
    }
}
