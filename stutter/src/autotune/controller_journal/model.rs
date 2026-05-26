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

pub const CONTROLLER_JOURNAL_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ControllerJournalState {
    Clean,
    Planned,
    Preflighted,
    Applying,
    Applied,
    Verifying,
    Measuring,
    Keeping,
    Reverting,
    Reverted,
    Faulted,
}

impl ControllerJournalState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::Planned => "planned",
            Self::Preflighted => "preflighted",
            Self::Applying => "applying",
            Self::Applied => "applied",
            Self::Verifying => "verifying",
            Self::Measuring => "measuring",
            Self::Keeping => "keeping",
            Self::Reverting => "reverting",
            Self::Reverted => "reverted",
            Self::Faulted => "faulted",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControllerJournalRecord {
    pub schema_version: u32,
    pub state: ControllerJournalState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experiment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workload_identity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_identity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restore_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify_result: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<DaemonMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safety_class: Option<SafetyClass>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollback_token: Option<RollbackToken>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase_started_unix_nanos: Option<u128>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_unix_nanos: Option<u128>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fault_reason: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControllerJournalActionMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workload_identity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_identity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restore_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify_result: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<DaemonMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safety_class: Option<SafetyClass>,
}

impl ControllerJournalActionMetadata {
    pub fn with_candidate(mut self, candidate: impl Into<String>) -> Self {
        self.candidate = Some(candidate.into());
        self
    }

    pub fn with_workload_identity(mut self, workload_identity: impl Into<String>) -> Self {
        self.workload_identity = Some(workload_identity.into());
        self
    }

    pub fn with_target_identity(mut self, target_identity: impl Into<String>) -> Self {
        self.target_identity = Some(target_identity.into());
        self
    }

    pub fn with_restore_command(mut self, restore_command: impl Into<String>) -> Self {
        self.restore_command = Some(restore_command.into());
        self
    }

    pub fn with_verify_result(mut self, verify_result: impl Into<String>) -> Self {
        self.verify_result = Some(verify_result.into());
        self
    }

    pub fn with_mode(mut self, mode: DaemonMode) -> Self {
        self.mode = Some(mode);
        self
    }

    pub fn with_safety_class(mut self, safety_class: SafetyClass) -> Self {
        self.safety_class = Some(safety_class);
        self
    }
}

impl ControllerJournalRecord {
    pub fn applying(experiment_id: impl Into<String>, action_id: impl Into<String>) -> Self {
        Self::for_phase(
            ControllerJournalState::Applying,
            experiment_id,
            action_id,
            None,
        )
    }

    pub fn applied(
        experiment_id: impl Into<String>,
        action_id: impl Into<String>,
        rollback_token: RollbackToken,
    ) -> Self {
        Self::for_phase(
            ControllerJournalState::Applied,
            experiment_id,
            action_id,
            Some(rollback_token),
        )
    }

    pub fn clean() -> Self {
        Self {
            schema_version: CONTROLLER_JOURNAL_SCHEMA_VERSION,
            state: ControllerJournalState::Clean,
            experiment_id: None,
            action_id: None,
            candidate: None,
            workload_identity: None,
            target_identity: None,
            restore_command: None,
            verify_result: None,
            mode: None,
            safety_class: None,
            rollback_token: None,
            phase_started_unix_nanos: None,
            updated_unix_nanos: None,
            fault_reason: None,
        }
    }

    pub fn for_phase(
        state: ControllerJournalState,
        experiment_id: impl Into<String>,
        action_id: impl Into<String>,
        rollback_token: Option<RollbackToken>,
    ) -> Self {
        Self {
            schema_version: CONTROLLER_JOURNAL_SCHEMA_VERSION,
            state,
            experiment_id: Some(experiment_id.into()),
            action_id: Some(action_id.into()),
            candidate: None,
            workload_identity: None,
            target_identity: None,
            restore_command: None,
            verify_result: None,
            mode: None,
            safety_class: None,
            rollback_token,
            phase_started_unix_nanos: Some(crate::audit::unix_nanos_now()),
            updated_unix_nanos: Some(crate::audit::unix_nanos_now()),
            fault_reason: None,
        }
    }

    pub fn with_candidate(mut self, candidate: impl Into<String>) -> Self {
        self.candidate = Some(candidate.into());
        self
    }

    pub fn with_workload_identity(mut self, workload_identity: impl Into<String>) -> Self {
        self.workload_identity = Some(workload_identity.into());
        self
    }

    pub fn with_target_identity(mut self, target_identity: impl Into<String>) -> Self {
        self.target_identity = Some(target_identity.into());
        self
    }

    pub fn with_restore_command(mut self, restore_command: impl Into<String>) -> Self {
        self.restore_command = Some(restore_command.into());
        self
    }

    pub fn with_verify_result(mut self, verify_result: impl Into<String>) -> Self {
        self.verify_result = Some(verify_result.into());
        self
    }

    pub fn with_mode(mut self, mode: DaemonMode) -> Self {
        self.mode = Some(mode);
        self
    }

    pub fn with_safety_class(mut self, safety_class: SafetyClass) -> Self {
        self.safety_class = Some(safety_class);
        self
    }

    pub fn with_fault_reason(mut self, fault_reason: impl Into<String>) -> Self {
        self.fault_reason = Some(fault_reason.into());
        self
    }

    pub fn with_metadata(mut self, metadata: ControllerJournalActionMetadata) -> Self {
        if metadata.candidate.is_some() {
            self.candidate = metadata.candidate;
        }
        if metadata.workload_identity.is_some() {
            self.workload_identity = metadata.workload_identity;
        }
        if metadata.target_identity.is_some() {
            self.target_identity = metadata.target_identity;
        }
        if metadata.restore_command.is_some() {
            self.restore_command = metadata.restore_command;
        }
        if metadata.verify_result.is_some() {
            self.verify_result = metadata.verify_result;
        }
        if metadata.mode.is_some() {
            self.mode = metadata.mode;
        }
        if metadata.safety_class.is_some() {
            self.safety_class = metadata.safety_class;
        }
        self
    }

    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub fn state(&self) -> ControllerJournalState {
        self.state
    }

    pub fn is_clean(&self) -> bool {
        self.state == ControllerJournalState::Clean
    }

    pub fn experiment_action(&self) -> Option<(&str, &str)> {
        Some((self.experiment_id.as_deref()?, self.action_id.as_deref()?))
    }

    pub fn rollback_token(&self) -> Option<&RollbackToken> {
        self.rollback_token.as_ref()
    }

    pub fn is_active_experiment_state(&self) -> bool {
        matches!(
            self.state,
            ControllerJournalState::Applying
                | ControllerJournalState::Applied
                | ControllerJournalState::Verifying
                | ControllerJournalState::Measuring
                | ControllerJournalState::Keeping
                | ControllerJournalState::Reverting
                | ControllerJournalState::Faulted
        )
    }

    pub fn may_have_mutated_system(&self) -> bool {
        matches!(
            self.state,
            ControllerJournalState::Applying
                | ControllerJournalState::Applied
                | ControllerJournalState::Verifying
                | ControllerJournalState::Measuring
                | ControllerJournalState::Keeping
                | ControllerJournalState::Reverting
                | ControllerJournalState::Faulted
        )
    }
}

pub fn default_controller_journal_path() -> PathBuf {
    let mut path = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    path.push(".local");
    path.push("state");
    path.push("stutter");
    path.push("autotune");
    path.push("controller_journal.json");
    path
}

pub fn journal_process_identity(
    pid: u32,
    starttime_ticks: Option<u64>,
    active_task_count: Option<usize>,
) -> String {
    let starttime = starttime_ticks
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unknown".to_owned());
    let mut identity = format!("pid:{pid}:starttime:{starttime}");
    if let Some(active_task_count) = active_task_count {
        identity.push_str(&format!(":active_tasks:{active_task_count}"));
    }
    identity
}

pub fn write_controller_journal_record(
    path: &Path,
    record: &ControllerJournalRecord,
) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create autotune controller journal directory {}",
                parent.display()
            )
        })?;
    }

    let (temporary_path, mut file) = create_unique_journal_temp_file(path)?;
    serde_json::to_writer_pretty(&mut file, record).with_context(|| {
        format!(
            "failed to serialize autotune controller journal {}",
            temporary_path.display()
        )
    })?;
    file.write_all(b"\n").with_context(|| {
        format!(
            "failed to terminate autotune controller journal {}",
            temporary_path.display()
        )
    })?;
    file.sync_all().with_context(|| {
        format!(
            "failed to sync autotune controller journal temp file {}",
            temporary_path.display()
        )
    })?;
    drop(file);

    fs::rename(&temporary_path, path).with_context(|| {
        format!(
            "failed to atomically replace autotune controller journal {} with {}",
            path.display(),
            temporary_path.display()
        )
    })?;

    sync_parent_directory(path)?;

    Ok(())
}

pub fn read_controller_journal(path: &Path) -> anyhow::Result<ControllerJournalRecord> {
    if !path.exists() {
        return Ok(ControllerJournalRecord::clean());
    }

    let file = fs::File::open(path).with_context(|| {
        format!(
            "failed to open autotune controller journal {}",
            path.display()
        )
    })?;

    let record: ControllerJournalRecord = serde_json::from_reader(file).with_context(|| {
        format!(
            "failed to parse autotune controller journal {}",
            path.display()
        )
    })?;

    if record.schema_version() != CONTROLLER_JOURNAL_SCHEMA_VERSION {
        anyhow::bail!(
            "unsupported autotune controller journal schema_version={} in {}",
            record.schema_version(),
            path.display()
        );
    }

    Ok(record)
}

pub fn write_controller_journal_phase_with_metadata(
    path: &Path,
    state: ControllerJournalState,
    experiment_id: &str,
    action_id: &str,
    rollback_token: Option<RollbackToken>,
    metadata: ControllerJournalActionMetadata,
) -> anyhow::Result<ControllerJournalRecord> {
    let record =
        ControllerJournalRecord::for_phase(state, experiment_id, action_id, rollback_token)
            .with_metadata(metadata);
    write_controller_journal_record(path, &record)?;
    Ok(record)
}

pub fn write_controller_journal_applying(
    path: &Path,
    experiment_id: &str,
    action_id: &str,
) -> anyhow::Result<ControllerJournalRecord> {
    let record = ControllerJournalRecord::applying(experiment_id, action_id);
    write_controller_journal_record(path, &record)?;
    Ok(record)
}

pub fn write_controller_journal_applying_with_metadata(
    path: &Path,
    experiment_id: &str,
    action_id: &str,
    metadata: ControllerJournalActionMetadata,
) -> anyhow::Result<ControllerJournalRecord> {
    write_controller_journal_phase_with_metadata(
        path,
        ControllerJournalState::Applying,
        experiment_id,
        action_id,
        None,
        metadata,
    )
}

pub fn write_controller_journal_applied(
    path: &Path,
    experiment_id: &str,
    action_id: &str,
    rollback_token: RollbackToken,
) -> anyhow::Result<ControllerJournalRecord> {
    let record = ControllerJournalRecord::applied(experiment_id, action_id, rollback_token);
    write_controller_journal_record(path, &record)?;
    Ok(record)
}

pub fn write_controller_journal_applied_with_metadata(
    path: &Path,
    experiment_id: &str,
    action_id: &str,
    rollback_token: RollbackToken,
    metadata: ControllerJournalActionMetadata,
) -> anyhow::Result<ControllerJournalRecord> {
    write_controller_journal_phase_with_metadata(
        path,
        ControllerJournalState::Applied,
        experiment_id,
        action_id,
        Some(rollback_token),
        metadata,
    )
}

pub fn write_controller_journal_clean(path: &Path) -> anyhow::Result<ControllerJournalRecord> {
    let record = ControllerJournalRecord::clean();
    write_controller_journal_record(path, &record)?;
    Ok(record)
}

pub fn write_default_controller_journal_applying(
    experiment_id: &str,
    action_id: &str,
) -> anyhow::Result<PathBuf> {
    let path = default_controller_journal_path();
    write_controller_journal_applying(&path, experiment_id, action_id)?;
    Ok(path)
}

pub fn write_default_controller_journal_applied(
    experiment_id: &str,
    action_id: &str,
    rollback_token: RollbackToken,
) -> anyhow::Result<PathBuf> {
    let path = default_controller_journal_path();
    write_controller_journal_applied(&path, experiment_id, action_id, rollback_token)?;
    Ok(path)
}

pub fn write_default_controller_journal_clean() -> anyhow::Result<PathBuf> {
    let path = default_controller_journal_path();
    write_controller_journal_clean(&path)?;
    Ok(path)
}

fn create_unique_journal_temp_file(path: &Path) -> anyhow::Result<(PathBuf, fs::File)> {
    for _ in 0..16 {
        let temporary_path = unique_temporary_journal_path(path);
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
                        "failed to create autotune controller journal temp file {}",
                        temporary_path.display()
                    )
                });
            }
        }
    }

    anyhow::bail!(
        "failed to create a unique autotune controller journal temp file for {} after repeated collisions",
        path.display()
    );
}

fn unique_temporary_journal_path(path: &Path) -> PathBuf {
    let mut temporary_path = path.to_path_buf();
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("controller_journal.json");
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
                "failed to open autotune controller journal directory {} for sync",
                parent.display()
            )
        })?;
        directory.sync_all().with_context(|| {
            format!(
                "failed to sync autotune controller journal directory {}",
                parent.display()
            )
        })?;
    }

    Ok(())
}

