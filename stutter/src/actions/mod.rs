#![allow(dead_code)]

#[cfg(test)]
pub mod fake_action;

pub mod cgroup;
pub mod cpu_affinity;
pub mod cpu_power;
pub mod gpu_power;
pub mod nice;

pub mod ioprio;
pub mod irq_affinity;
pub mod uclamp;
pub mod vm_knobs;

pub mod runner;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionPhase {
    Preflight,
    DryRun,
    Apply,
    Verify,
    Rollback,
    EmergencyRollback,
}

impl ActionPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Preflight => "preflight",
            Self::DryRun => "dry_run",
            Self::Apply => "apply",
            Self::Verify => "verify",
            Self::Rollback => "rollback",
            Self::EmergencyRollback => "emergency_rollback",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActionError {
    PreflightFailure {
        message: String,
    },
    DryRunFailure {
        message: String,
    },
    ApplyFailure {
        message: String,
    },
    VerifyFailure {
        message: String,
    },
    VerifyFailureRollbackCompleted {
        verify_error: String,
    },
    RollbackFailure {
        message: String,
    },
    EmergencyRollbackFailure {
        verify_error: String,
        rollback_error: String,
    },
    PolicyRejected {
        message: String,
    },
}

impl ActionError {
    pub fn preflight(error: impl std::fmt::Display) -> Self {
        Self::PreflightFailure {
            message: error.to_string(),
        }
    }

    pub fn dry_run(error: impl std::fmt::Display) -> Self {
        Self::DryRunFailure {
            message: error.to_string(),
        }
    }

    pub fn apply(error: impl std::fmt::Display) -> Self {
        Self::ApplyFailure {
            message: error.to_string(),
        }
    }

    pub fn verify(error: impl std::fmt::Display) -> Self {
        Self::VerifyFailure {
            message: error.to_string(),
        }
    }

    pub fn verify_rollback_completed(verify_error: impl std::fmt::Display) -> Self {
        Self::VerifyFailureRollbackCompleted {
            verify_error: verify_error.to_string(),
        }
    }

    pub fn rollback(error: impl std::fmt::Display) -> Self {
        Self::RollbackFailure {
            message: error.to_string(),
        }
    }

    pub fn emergency_rollback(
        verify_error: impl std::fmt::Display,
        rollback_error: impl std::fmt::Display,
    ) -> Self {
        Self::EmergencyRollbackFailure {
            verify_error: verify_error.to_string(),
            rollback_error: rollback_error.to_string(),
        }
    }

    pub fn policy_rejected(error: impl std::fmt::Display) -> Self {
        Self::PolicyRejected {
            message: error.to_string(),
        }
    }

    pub fn phase(&self) -> ActionPhase {
        match self {
            Self::PreflightFailure { .. } => ActionPhase::Preflight,
            Self::DryRunFailure { .. } => ActionPhase::DryRun,
            Self::ApplyFailure { .. } => ActionPhase::Apply,
            Self::VerifyFailure { .. } | Self::VerifyFailureRollbackCompleted { .. } => {
                ActionPhase::Verify
            }
            Self::RollbackFailure { .. } => ActionPhase::Rollback,
            Self::EmergencyRollbackFailure { .. } => ActionPhase::EmergencyRollback,
            Self::PolicyRejected { .. } => ActionPhase::Preflight,
        }
    }

    pub fn category(&self) -> &'static str {
        match self {
            Self::PreflightFailure { .. } => "preflight_failure",
            Self::DryRunFailure { .. } => "dry_run_failure",
            Self::ApplyFailure { .. } => "apply_failure",
            Self::VerifyFailure { .. } => "verify_failure",
            Self::VerifyFailureRollbackCompleted { .. } => "verify_failure_rollback_completed",
            Self::RollbackFailure { .. } => "rollback_failure",
            Self::EmergencyRollbackFailure { .. } => "emergency_rollback_failure",
            Self::PolicyRejected { .. } => "policy_rejected",
        }
    }

    pub fn human_message(&self) -> String {
        match self {
            Self::PreflightFailure { message } => format!("preflight failed: {message}"),
            Self::DryRunFailure { message } => format!("dry run failed: {message}"),
            Self::ApplyFailure { message } => format!("apply failed: {message}"),
            Self::VerifyFailure { message } => format!("verify failed: {message}"),
            Self::VerifyFailureRollbackCompleted { verify_error } => {
                format!("verify failed; rollback completed: {verify_error}")
            }
            Self::RollbackFailure { message } => format!("rollback failed: {message}"),
            Self::EmergencyRollbackFailure {
                verify_error,
                rollback_error,
            } => format!(
                "verify failed; emergency rollback failed: verify error: {verify_error}; rollback error: {rollback_error}"
            ),
            Self::PolicyRejected { message } => format!("policy rejected: {message}"),
        }
    }
}

impl std::fmt::Display for ActionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.human_message())
    }
}

impl std::error::Error for ActionError {}

pub type ActionResult<T> = anyhow::Result<T>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum SafetyClass {
    #[default]
    ObserveOnly,
    ReversibleLowRisk,
    ReversibleMediumRisk,
    HighRisk,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionId(pub String);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionWarning {
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionState {
    pub applied: bool,
    pub affected_tasks: usize,
    pub checked_tasks: usize,
    pub pending_changes: usize,
    pub warnings: Vec<ActionWarning>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskIdentity {
    pub tid: u32,
    pub process_pid: Option<u32>,
    pub comm: Option<String>,
    pub starttime_ticks: Option<u64>,
}

impl TaskIdentity {
    pub fn from_task_info(task: &crate::process_tree::TaskInfo) -> Self {
        Self {
            tid: task.tid,
            process_pid: Some(task.process_pid),
            comm: Some(task.comm.clone()),
            starttime_ticks: task.task_starttime_ticks,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NiceRestoreRecord {
    pub tid: u32,
    pub original_nice: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UclampRestoreRecord {
    pub tid: u32,
    pub original_util_min: u32,
    pub original_util_max: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IrqAffinityRestoreRecord {
    pub irq: u32,
    pub device_hint: String,
    pub original_smp_affinity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IoPrioRestoreRecord {
    pub tid: u32,
    pub original_ioprio: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VmKnobRestoreRecord {
    pub path: PathBuf,
    pub original_value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GpuPowerRestoreRecord {
    pub path: PathBuf,
    pub original_value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CpuPowerRestoreRecord {
    pub path: PathBuf,
    pub original_value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CgroupRestoreRecord {
    pub pid: u32,
    pub original_cgroup: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionOutcome {
    pub action_id: ActionId,
    pub safety_class: SafetyClass,
    pub dry_run: bool,
    pub preflight_warnings: Vec<ActionWarning>,
    pub state: ActionState,
    pub rollback: Option<RollbackToken>,
    pub started_unix_nanos: u128,
    pub finished_unix_nanos: u128,
}

pub struct RollbackRegistry {
    handlers: Vec<Box<dyn RollbackHandler>>,
}

impl RollbackRegistry {
    pub fn new() -> Self {
        Self {
            handlers: Vec::new(),
        }
    }

    pub fn register<H>(&mut self, handler: H)
    where
        H: RollbackHandler + 'static,
    {
        self.handlers.push(Box::new(handler));
    }

    pub fn handlers(&self) -> &[Box<dyn RollbackHandler>] {
        &self.handlers
    }
}

impl Default for RollbackRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub trait RollbackHandler {
    fn id(&self) -> &'static str;
    fn discover(&self) -> anyhow::Result<Vec<RollbackCandidate>>;
    fn dry_run(&self, candidate: &RollbackCandidate) -> anyhow::Result<RollbackPreview>;
    fn restore(&self, candidate: RollbackCandidate) -> anyhow::Result<RollbackResult>;
}

#[derive(Debug, Clone)]
pub struct RollbackCandidate {
    pub handler_id: &'static str,
    pub restore_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct RollbackPreview {
    pub handler_id: &'static str,
    pub restore_path: PathBuf,
    pub affected_tasks: usize,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct RollbackResult {
    pub handler_id: &'static str,
    pub restore_path: PathBuf,
    pub restored: usize,
    pub skipped_dead: usize,
    pub skipped_identity_mismatch: usize,
    pub legacy_unverified: usize,
    pub errors: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct RestoreAllInput {
    pub dry_run: bool,
}

#[derive(Debug, Clone, Default)]
pub struct RestoreAllSummary {
    pub restored_total: usize,
    pub skipped_dead: usize,
    pub skipped_identity_mismatch: usize,
    pub legacy_unverified: usize,
    pub errors: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind")]
pub enum RollbackToken {
    #[serde(rename = "cpu-affinity-restore-file")]
    CpuAffinityRestoreFile {
        #[serde(rename = "restore_path")]
        path: PathBuf,
        affected_tasks: usize,
    },
    NiceRestore {
        records: Vec<NiceRestoreRecord>,
    },
    IrqAffinityRestore {
        records: Vec<IrqAffinityRestoreRecord>,
    },
    IoPrioRestore {
        records: Vec<IoPrioRestoreRecord>,
    },
    UclampRestore {
        records: Vec<UclampRestoreRecord>,
    },
    CgroupRestore {
        records: Vec<CgroupRestoreRecord>,
    },
    CpuPowerRestore {
        records: Vec<CpuPowerRestoreRecord>,
    },
    VmKnobRestore {
        records: Vec<VmKnobRestoreRecord>,
    },
    GpuPowerRestore {
        records: Vec<GpuPowerRestoreRecord>,
    },
    SysfsRestore {
        path: PathBuf,
        original_value: String,
    },
}

impl RollbackToken {
    pub fn affected_tasks(&self) -> usize {
        match self {
            Self::CpuAffinityRestoreFile { affected_tasks, .. } => *affected_tasks,
            Self::NiceRestore { records } => records.len(),
            Self::IrqAffinityRestore { records } => records.len(),
            Self::IoPrioRestore { records } => records.len(),
            Self::UclampRestore { records } => records.len(),
            Self::CgroupRestore { records } => records.len(),
            Self::CpuPowerRestore { records } => records.len(),
            Self::VmKnobRestore { records } => records.len(),
            Self::GpuPowerRestore { records } => records.len(),
            Self::SysfsRestore { .. } => 1,
        }
    }

    pub fn restore_path(&self) -> Option<&PathBuf> {
        match self {
            Self::CpuAffinityRestoreFile { path, .. } => Some(path),
            Self::SysfsRestore { path, .. } => Some(path),
            Self::NiceRestore { .. }
            | Self::IrqAffinityRestore { .. }
            | Self::IoPrioRestore { .. }
            | Self::UclampRestore { .. }
            | Self::CgroupRestore { .. }
            | Self::CpuPowerRestore { .. }
            | Self::GpuPowerRestore { .. }
            | Self::VmKnobRestore { .. } => None,
        }
    }
}

pub trait TuningAction {
    fn id(&self) -> ActionId;
    fn describe(&self) -> String;
    fn safety_class(&self) -> SafetyClass;

    fn descriptor(&self) -> crate::daemon_policy::ActionDescriptor {
        let action_id = self.id();
        crate::daemon_policy::ActionDescriptor {
            action_kind: action_id.0.clone(),
            action_id,
            safety_class: self.safety_class(),
            effect_scope: crate::daemon_policy::ActionEffectScope::LocalProcessTree,
            rollback: crate::daemon_policy::RollbackRequirement::RequiredBeforeApply,
            persistent_effect: false,
            touches_system_wide_state: false,
            requires_explicit_target: true,
            confidence: None,
        }
    }

    fn preflight(&self) -> ActionResult<Vec<ActionWarning>>;
    fn dry_run(&self) -> ActionResult<ActionState>;
    fn apply(&self) -> ActionResult<RollbackToken>;
    fn verify(&self) -> ActionResult<ActionState>;
    fn rollback(&self, token: &RollbackToken) -> ActionResult<()>;
}
