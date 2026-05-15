use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::{
    actions::{RollbackToken, SafetyClass},
    daemon::{health::SystemHealthSnapshot, policy::DaemonMode},
    metadata::SystemMetadata,
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
    Paused,
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
            Self::Paused => "paused",
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
        matches!(
            self,
            Self::Disabled | Self::Paused | Self::Faulted | Self::Shutdown
        )
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
    #[serde(default)]
    pub health: SystemHealthSnapshot,
    pub degraded: Vec<DaemonDegradedStatus>,
    pub faulted: Option<DaemonFaultState>,
    #[serde(default)]
    pub profile_memory: DaemonProfileMemory,
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
            health: SystemHealthSnapshot::default(),
            degraded: Vec::new(),
            faulted: None,
            profile_memory: DaemonProfileMemory::default(),
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
    #[serde(default)]
    pub candidate_count: Option<usize>,
    #[serde(default)]
    pub top_denied_reason: Option<String>,
    #[serde(default)]
    pub situation: Option<String>,
    #[serde(default)]
    pub focus_kind: Option<String>,
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

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonProfileMemory {
    pub profiles: Vec<DaemonWorkloadProfile>,
}

impl DaemonProfileMemory {
    pub fn forget_matching(
        &mut self,
        workload_identity_hash: Option<&str>,
        candidate_name: Option<&str>,
        all: bool,
    ) -> Vec<DaemonWorkloadProfile> {
        let mut removed = Vec::new();
        let mut retained = Vec::new();

        for profile in self.profiles.drain(..) {
            let matches = all
                || workload_identity_hash
                    .is_some_and(|hash| profile.workload_identity_hash == hash);
            let candidate_matches = candidate_name
                .map(|name| profile.candidate_name == name || profile.action_id == name)
                .unwrap_or(true);

            if matches && candidate_matches {
                removed.push(profile);
            } else {
                retained.push(profile);
            }
        }

        self.profiles = retained;
        removed
    }

    pub fn sorted_profiles(&self) -> Vec<DaemonWorkloadProfile> {
        let mut profiles = self.profiles.clone();
        profiles.sort_by(|left, right| {
            left.workload_identity_hash
                .cmp(&right.workload_identity_hash)
                .then_with(|| left.candidate_name.cmp(&right.candidate_name))
                .then_with(|| right.kept_unix_nanos.cmp(&left.kept_unix_nanos))
        });
        profiles
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonWorkloadProfile {
    pub workload_identity_hash: String,
    pub workload_label: Option<String>,
    pub candidate_name: String,
    pub action_id: String,
    pub action_kind: String,
    pub safety_class: SafetyClass,
    pub kept_unix_nanos: u128,
    pub last_validated_unix_nanos: Option<u128>,
    pub baseline_score_total: Option<u64>,
    pub candidate_score_total: Option<u64>,
    pub score_delta: i64,
    pub confidence_milli: u16,
    pub environment: DaemonProfileEnvironment,
    #[serde(default)]
    pub partition: DaemonProfilePartition,
}

impl DaemonWorkloadProfile {
    pub fn validation(
        &self,
        current_environment: &DaemonProfileEnvironment,
        now_unix_nanos: u128,
    ) -> DaemonProfileValidation {
        let mut reason_codes = Vec::<String>::new();

        if self.workload_identity_hash.trim().is_empty() {
            reason_codes.push("missing_workload_identity_hash".to_owned());
        }

        push_profile_environment_mismatch(
            &mut reason_codes,
            "hardware_changed",
            self.environment.hardware_fingerprint.as_deref(),
            current_environment.hardware_fingerprint.as_deref(),
        );
        push_profile_environment_mismatch(
            &mut reason_codes,
            "kernel_changed",
            self.environment.kernel_version.as_deref(),
            current_environment.kernel_version.as_deref(),
        );
        push_profile_environment_mismatch(
            &mut reason_codes,
            "cpu_topology_changed",
            self.environment.cpu_topology_hash.as_deref(),
            current_environment.cpu_topology_hash.as_deref(),
        );
        push_profile_environment_mismatch(
            &mut reason_codes,
            "scheduler_changed",
            self.environment.scheduler_label.as_deref(),
            current_environment.scheduler_label.as_deref(),
        );
        push_profile_environment_mismatch(
            &mut reason_codes,
            "scx_state_changed",
            self.environment.scx_state.as_deref(),
            current_environment.scx_state.as_deref(),
        );
        push_profile_environment_mismatch(
            &mut reason_codes,
            "scx_ops_changed",
            self.environment.scx_ops.as_deref(),
            current_environment.scx_ops.as_deref(),
        );

        let age_nanos = now_unix_nanos.saturating_sub(
            self.last_validated_unix_nanos
                .unwrap_or(self.kept_unix_nanos),
        );
        let mut confidence_milli = self.confidence_milli.min(1000);

        if age_nanos > PROFILE_REVALIDATE_AFTER_NANOS {
            reason_codes.push("revalidation_due".to_owned());
            confidence_milli = confidence_milli.saturating_mul(800) / 1000;
        }
        if age_nanos > PROFILE_MAX_TRUST_AGE_NANOS {
            reason_codes.push("profile_too_old".to_owned());
            confidence_milli = confidence_milli.min(250);
        }

        let mut unique_reasons = BTreeSet::new();
        reason_codes.retain(|reason| unique_reasons.insert(reason.clone()));
        let valid = !reason_codes
            .iter()
            .any(|reason| reason != "revalidation_due");

        DaemonProfileValidation {
            valid,
            confidence_milli,
            reason_codes,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonProfilePartition {
    pub power_source: Option<String>,
    pub display_refresh_millihz: Option<u32>,
    pub fps_cap: Option<u32>,
    pub graphics_settings_hash: Option<String>,
    pub scheduler_label: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonProfileEnvironment {
    pub hardware_fingerprint: Option<String>,
    pub kernel_version: Option<String>,
    pub mesa_driver_version: Option<String>,
    pub cpu_topology_hash: Option<String>,
    pub scheduler_label: Option<String>,
    pub scx_state: Option<String>,
    pub scx_ops: Option<String>,
}

impl DaemonProfileEnvironment {
    pub fn current() -> Self {
        Self::from_system_metadata(&crate::metadata::collect_system_metadata())
    }

    pub fn from_system_metadata(metadata: &SystemMetadata) -> Self {
        let cpu_topology_hash = daemon_profile_cpu_topology_hash(metadata);
        let hardware_fingerprint = daemon_profile_stable_hash([
            metadata.cpu_possible.as_deref().unwrap_or("-"),
            metadata.cpu_online.as_deref().unwrap_or("-"),
            cpu_topology_hash.as_deref().unwrap_or("-"),
        ]);
        let scheduler_label = metadata
            .scx_ops
            .clone()
            .or_else(|| metadata.scx_state.clone())
            .filter(|value| !value.trim().is_empty());

        Self {
            hardware_fingerprint: Some(hardware_fingerprint),
            kernel_version: metadata
                .kernel_osrelease
                .clone()
                .or_else(|| metadata.kernel_version.clone()),
            mesa_driver_version: None,
            cpu_topology_hash,
            scheduler_label,
            scx_state: metadata.scx_state.clone(),
            scx_ops: metadata.scx_ops.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonProfileValidation {
    pub valid: bool,
    pub confidence_milli: u16,
    pub reason_codes: Vec<String>,
}

const PROFILE_REVALIDATE_AFTER_NANOS: u128 = 30 * 24 * 60 * 60 * 1_000_000_000;
const PROFILE_MAX_TRUST_AGE_NANOS: u128 = 180 * 24 * 60 * 60 * 1_000_000_000;

fn push_profile_environment_mismatch(
    reason_codes: &mut Vec<String>,
    reason_code: &str,
    stored: Option<&str>,
    current: Option<&str>,
) {
    let (Some(stored), Some(current)) = (stored, current) else {
        return;
    };

    if stored != current {
        reason_codes.push(reason_code.to_owned());
    }
}

fn daemon_profile_cpu_topology_hash(metadata: &SystemMetadata) -> Option<String> {
    if metadata.cpu_topology.is_empty()
        && metadata.cpu_online.is_none()
        && metadata.cpu_possible.is_none()
    {
        return None;
    }

    let mut parts = Vec::new();
    parts.push(format!(
        "online={}",
        metadata.cpu_online.as_deref().unwrap_or("-")
    ));
    parts.push(format!(
        "possible={}",
        metadata.cpu_possible.as_deref().unwrap_or("-")
    ));
    for cpu in &metadata.cpu_topology {
        parts.push(format!(
            "cpu={}:siblings={}:core={}:package={}",
            cpu.cpu,
            cpu.thread_siblings_list.as_deref().unwrap_or("-"),
            cpu.core_id.as_deref().unwrap_or("-"),
            cpu.physical_package_id.as_deref().unwrap_or("-")
        ));
    }

    Some(daemon_profile_stable_hash(parts.iter().map(String::as_str)))
}

pub(crate) fn daemon_profile_stable_hash<'a>(parts: impl IntoIterator<Item = &'a str>) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;

    for part in parts {
        for byte in part.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }

    format!("{hash:016x}")
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

        let (temporary_path, mut file) = create_unique_daemon_state_snapshot_temp_file(&self.path)?;
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
        drop(file);

        fs::rename(&temporary_path, &self.path).with_context(|| {
            format!(
                "failed to atomically replace daemon state snapshot {} with {}",
                self.path.display(),
                temporary_path.display()
            )
        })?;

        sync_parent_directory(&self.path)?;

        Ok(())
    }
}

fn create_unique_daemon_state_snapshot_temp_file(
    path: &Path,
) -> anyhow::Result<(PathBuf, fs::File)> {
    for _ in 0..16 {
        let temporary_path = unique_temporary_daemon_state_snapshot_path(path);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)
        {
            Ok(file) => return Ok((temporary_path, file)),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => {
                return Err(err).with_context(|| {
                    format!(
                        "failed to create daemon state snapshot temp file {}",
                        temporary_path.display()
                    )
                });
            }
        }
    }

    anyhow::bail!(
        "failed to create a unique daemon state snapshot temp file for {} after repeated collisions",
        path.display()
    );
}

fn unique_temporary_daemon_state_snapshot_path(path: &Path) -> PathBuf {
    let mut temporary_path = path.to_path_buf();
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("daemon_state.json");
    temporary_path.set_file_name(format!(
        "{file_name}.{}.{}.tmp",
        std::process::id(),
        crate::audit::unix_nanos_now()
    ));
    temporary_path
}

fn sync_parent_directory(path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        let directory = fs::File::open(parent).with_context(|| {
            format!(
                "failed to open daemon state snapshot directory {} for sync",
                parent.display()
            )
        })?;
        directory.sync_all().with_context(|| {
            format!(
                "failed to sync daemon state snapshot directory {}",
                parent.display()
            )
        })?;
    }

    Ok(())
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

    fn temporary_files_in(dir: &Path) -> Vec<PathBuf> {
        fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("tmp"))
            .collect()
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
        assert_eq!(DaemonPhase::Paused.lifecycle_label(), "paused");
        assert_eq!(DaemonPhase::Observe.lifecycle_label(), "observe");
        assert_eq!(DaemonPhase::Decide.lifecycle_label(), "decide");
        assert_eq!(DaemonPhase::Apply.lifecycle_label(), "apply");
        assert_eq!(DaemonPhase::Measure.lifecycle_label(), "measure");
        assert_eq!(DaemonPhase::Rollback.lifecycle_label(), "rollback");
        assert_eq!(DaemonPhase::Cooldown.lifecycle_label(), "cooldown");
        assert_eq!(DaemonPhase::Faulted.lifecycle_label(), "faulted");
        assert_eq!(DaemonPhase::Shutdown.lifecycle_label(), "shutdown");

        assert!(DaemonPhase::Disabled.is_terminal());
        assert!(DaemonPhase::Paused.is_terminal());
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
            (DaemonPhase::Paused, "\"paused\""),
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
            ("\"paused\"", DaemonPhase::Paused),
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
            profile_memory: DaemonProfileMemory {
                profiles: vec![DaemonWorkloadProfile {
                    workload_identity_hash: "workload-abc".to_owned(),
                    workload_label: Some("game".to_owned()),
                    candidate_name: "game-main".to_owned(),
                    action_id: "cpu-affinity-profile:game-main".to_owned(),
                    action_kind: "cpu_affinity_profile".to_owned(),
                    safety_class: SafetyClass::ReversibleLowRisk,
                    kept_unix_nanos: 300,
                    last_validated_unix_nanos: Some(300),
                    baseline_score_total: Some(1000),
                    candidate_score_total: Some(850),
                    score_delta: -150,
                    confidence_milli: 900,
                    environment: DaemonProfileEnvironment::default(),
                    partition: DaemonProfilePartition {
                        power_source: Some("ac".to_owned()),
                        scheduler_label: Some("scx_lavd".to_owned()),
                        ..DaemonProfilePartition::default()
                    },
                }],
            },
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
        assert_eq!(decoded.profile_memory.profiles.len(), 1);
        assert_eq!(
            decoded.profile_memory.profiles[0].workload_identity_hash,
            "workload-abc"
        );
    }

    #[test]
    fn daemon_state_defaults_new_runtime_fields_when_loading_older_snapshots() {
        let json = r#"{
            "schema_version": 1,
            "mode": "observe",
            "phase": "disabled",
            "cooldown_until_unix_nanos": null,
            "active_target": null,
            "active_experiment": null,
            "active_rollback": null,
            "last_decision": null,
            "degraded": [],
            "faulted": null
        }"#;

        let decoded: DaemonState = serde_json::from_str(json).unwrap();

        assert_eq!(
            decoded.health.state,
            crate::daemon::SystemHealthState::Healthy
        );
        assert!(decoded.health.ok_for_apply);
        assert!(decoded.profile_memory.profiles.is_empty());
    }

    #[test]
    fn profile_environment_hashes_kernel_scx_and_topology() {
        let metadata = SystemMetadata {
            kernel_osrelease: Some("6.12.0".to_owned()),
            cpu_online: Some("0-3".to_owned()),
            cpu_possible: Some("0-3".to_owned()),
            scx_state: Some("enabled".to_owned()),
            scx_ops: Some("scx_lavd".to_owned()),
            cpu_topology: vec![crate::metadata::CpuTopology {
                cpu: 0,
                thread_siblings_list: Some("0,2".to_owned()),
                core_id: Some("0".to_owned()),
                physical_package_id: Some("0".to_owned()),
            }],
            ..SystemMetadata::default()
        };

        let environment = DaemonProfileEnvironment::from_system_metadata(&metadata);
        let repeated = DaemonProfileEnvironment::from_system_metadata(&metadata);

        assert_eq!(environment, repeated);
        assert_eq!(environment.kernel_version.as_deref(), Some("6.12.0"));
        assert_eq!(environment.scheduler_label.as_deref(), Some("scx_lavd"));
        assert!(environment.hardware_fingerprint.is_some());
        assert!(environment.cpu_topology_hash.is_some());
    }

    #[test]
    fn workload_profile_validation_detects_environment_change_and_age() {
        let stored_environment = DaemonProfileEnvironment {
            hardware_fingerprint: Some("hardware-a".to_owned()),
            kernel_version: Some("6.12.0".to_owned()),
            cpu_topology_hash: Some("topology-a".to_owned()),
            scx_ops: Some("scx_lavd".to_owned()),
            scheduler_label: Some("scx_lavd".to_owned()),
            ..DaemonProfileEnvironment::default()
        };
        let current_environment = DaemonProfileEnvironment {
            hardware_fingerprint: Some("hardware-a".to_owned()),
            kernel_version: Some("6.13.0".to_owned()),
            cpu_topology_hash: Some("topology-b".to_owned()),
            scx_ops: Some("scx_bpfland".to_owned()),
            scheduler_label: Some("scx_bpfland".to_owned()),
            ..DaemonProfileEnvironment::default()
        };
        let profile = DaemonWorkloadProfile {
            workload_identity_hash: "workload-abc".to_owned(),
            workload_label: Some("game".to_owned()),
            candidate_name: "game-main".to_owned(),
            action_id: "cpu-affinity-profile:game-main".to_owned(),
            action_kind: "cpu_affinity_profile".to_owned(),
            safety_class: SafetyClass::ReversibleLowRisk,
            kept_unix_nanos: 100,
            last_validated_unix_nanos: Some(100),
            baseline_score_total: Some(1000),
            candidate_score_total: Some(850),
            score_delta: -150,
            confidence_milli: 900,
            environment: stored_environment,
            partition: DaemonProfilePartition::default(),
        };

        let validation = profile.validation(
            &current_environment,
            100 + PROFILE_REVALIDATE_AFTER_NANOS + 1,
        );

        assert!(!validation.valid);
        assert!(
            validation
                .reason_codes
                .contains(&"kernel_changed".to_owned())
        );
        assert!(
            validation
                .reason_codes
                .contains(&"cpu_topology_changed".to_owned())
        );
        assert!(
            validation
                .reason_codes
                .contains(&"scx_ops_changed".to_owned())
        );
        assert!(
            validation
                .reason_codes
                .contains(&"revalidation_due".to_owned())
        );
        assert!(validation.confidence_milli < 900);
    }

    #[test]
    fn profile_memory_forget_filters_by_workload_and_candidate() {
        let profile = |workload: &str, candidate: &str| DaemonWorkloadProfile {
            workload_identity_hash: workload.to_owned(),
            workload_label: Some(workload.to_owned()),
            candidate_name: candidate.to_owned(),
            action_id: format!("cpu-affinity-profile:{candidate}"),
            action_kind: "cpu_affinity_profile".to_owned(),
            safety_class: SafetyClass::ReversibleLowRisk,
            kept_unix_nanos: 1,
            last_validated_unix_nanos: Some(1),
            baseline_score_total: None,
            candidate_score_total: None,
            score_delta: 0,
            confidence_milli: 800,
            environment: DaemonProfileEnvironment::default(),
            partition: DaemonProfilePartition::default(),
        };
        let mut memory = DaemonProfileMemory {
            profiles: vec![
                profile("workload-a", "candidate-a"),
                profile("workload-a", "candidate-b"),
                profile("workload-b", "candidate-a"),
            ],
        };

        let removed = memory.forget_matching(Some("workload-a"), Some("candidate-a"), false);

        assert_eq!(removed.len(), 1);
        assert_eq!(memory.profiles.len(), 2);
        assert!(
            memory
                .profiles
                .iter()
                .all(|profile| profile.workload_identity_hash != "workload-a"
                    || profile.candidate_name != "candidate-a")
        );
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
        assert!(temporary_files_in(&dir).is_empty());

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn daemon_state_snapshot_writer_does_not_use_fixed_temp_path() {
        let dir = temp_dir("fixed-temp-sentinel");
        let path = dir.join("daemon_state.json");
        let fixed_temp_path = dir.join("daemon_state.json.tmp");
        fs::write(&fixed_temp_path, "sentinel").unwrap();

        let writer = DaemonStateSnapshotWriter::new(&path);
        let state = DaemonState {
            mode: DaemonMode::ApplyLowRisk,
            phase: DaemonPhase::Cooldown,
            ..DaemonState::default()
        };

        writer.write(&state).unwrap();

        assert_eq!(fs::read_to_string(&fixed_temp_path).unwrap(), "sentinel");
        assert_eq!(
            load_daemon_state(&path).unwrap().phase,
            DaemonPhase::Cooldown
        );

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
