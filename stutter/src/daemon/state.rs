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
    autotune::planner::PlannerSummary,
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
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
    #[serde(default = "default_active_autotune_mode")]
    pub mode: DaemonMode,
    pub safety_class: SafetyClass,
    pub started_unix_nanos: Option<u128>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonRollbackState {
    pub action_id: String,
    #[serde(default = "default_active_autotune_mode")]
    pub mode: DaemonMode,
    #[serde(default = "default_active_autotune_safety_class")]
    pub safety_class: SafetyClass,
    pub rollback_available: bool,
    pub token: Option<RollbackToken>,
    pub manual_restore_command: Option<String>,
}

fn default_active_autotune_mode() -> DaemonMode {
    DaemonMode::ApplyLowRisk
}

fn default_active_autotune_safety_class() -> SafetyClass {
    SafetyClass::ReversibleLowRisk
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DaemonDecisionState {
    pub decision: String,
    pub reason: String,
    pub unix_nanos: Option<u128>,
    #[serde(default, alias = "diagnostic_score_total")]
    pub diagnostic_current_raw_score_total: Option<u64>,
    #[serde(default)]
    pub candidate_count: Option<usize>,
    #[serde(default)]
    pub top_denied_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planner: Option<PlannerSummary>,
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
    pub diagnostic_baseline_raw_score_total: Option<u64>,
    #[serde(alias = "diagnostic_candidate_diagnostic_score_total")]
    pub diagnostic_candidate_raw_score_total: Option<u64>,
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
mod tests;
