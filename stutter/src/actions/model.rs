use std::path::PathBuf;

use serde::{Deserialize, Serialize};
pub use stutter_core::ids::{ActionId, Pid, Tid};

use crate::actions::token::RollbackToken;

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum SafetyClass {
    #[default]
    ObserveOnly,
    ReversibleLowRisk,
    ReversibleMediumRisk,
    HighRisk,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionWarning {
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionBlocker {
    pub message: String,
}

impl ActionBlocker {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionPreflightReport {
    pub action_id: ActionId,
    pub blockers: Vec<ActionBlocker>,
    pub warnings: Vec<ActionWarning>,
}

impl ActionPreflightReport {
    pub fn passed(action_id: ActionId, warnings: Vec<ActionWarning>) -> Self {
        Self {
            action_id,
            blockers: Vec::new(),
            warnings,
        }
    }

    pub fn blocked(action_id: ActionId, blocker: impl Into<String>) -> Self {
        Self {
            action_id,
            blockers: vec![ActionBlocker::new(blocker)],
            warnings: Vec::new(),
        }
    }

    pub fn from_preflight_result<E>(
        action_id: ActionId,
        result: Result<Vec<ActionWarning>, E>,
    ) -> Self
    where
        E: std::fmt::Display,
    {
        match result {
            Ok(warnings) => Self::passed(action_id, warnings),
            Err(err) => Self::blocked(action_id, err.to_string()),
        }
    }

    pub fn is_blocked(&self) -> bool {
        !self.blockers.is_empty()
    }

    pub fn blocker_messages(&self) -> String {
        self.blockers
            .iter()
            .map(|blocker| blocker.message.as_str())
            .collect::<Vec<_>>()
            .join("; ")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionState {
    pub applied: bool,
    pub affected_tasks: usize,
    pub checked_tasks: usize,
    pub pending_changes: usize,
    pub warnings: Vec<ActionWarning>,
}

// These task identifiers sit on JSON rollback-token serialization boundaries.
// Keep the Rust model typed and rely on the transparent serde representation of
// `Tid` and `Pid` so persisted rollback tokens retain their legacy numeric shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskIdentity {
    pub tid: Tid,
    pub process_pid: Option<Pid>,
    pub comm: Option<String>,
    pub starttime_ticks: Option<u64>,
}

impl TaskIdentity {
    pub fn from_task_info(task: &crate::process_tree::TaskInfo) -> Self {
        Self {
            tid: task.task_id(),
            process_pid: Some(task.process_id()),
            comm: Some(task.comm.clone()),
            starttime_ticks: task.task_starttime_ticks,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskRestoreIdentity {
    pub tid: Tid,
    #[serde(default)]
    pub process_pid: Option<Pid>,
    #[serde(default, alias = "task_starttime_ticks")]
    pub starttime_ticks: Option<u64>,
    #[serde(default)]
    pub comm: Option<String>,
    #[serde(default)]
    pub exe: Option<PathBuf>,
    #[serde(default)]
    pub process_starttime_ticks: Option<u64>,
}

impl TaskRestoreIdentity {
    pub fn from_task_info(task: &crate::process_tree::TaskInfo) -> Self {
        Self {
            tid: task.task_id(),
            process_pid: Some(task.process_id()),
            starttime_ticks: task.task_starttime_ticks,
            comm: Some(task.comm.clone()),
            exe: None,
            process_starttime_ticks: task.process_starttime_ticks,
        }
    }

    pub fn from_task_identity(task: &TaskIdentity) -> Self {
        Self {
            tid: task.tid,
            process_pid: task.process_pid,
            starttime_ticks: task.starttime_ticks,
            comm: task.comm.clone(),
            exe: None,
            process_starttime_ticks: None,
        }
    }

    pub fn observed(
        tid: u32,
        process_pid: Option<u32>,
        comm: Option<String>,
        starttime_ticks: Option<u64>,
        exe: Option<PathBuf>,
    ) -> Self {
        Self {
            tid: tid.into(),
            process_pid: process_pid.map(Into::into),
            starttime_ticks,
            comm,
            exe,
            process_starttime_ticks: None,
        }
    }

    pub fn legacy(tid: u32) -> Self {
        Self {
            tid: tid.into(),
            process_pid: None,
            starttime_ticks: None,
            comm: None,
            exe: None,
            process_starttime_ticks: None,
        }
    }
}

fn zero_tid() -> Tid {
    Tid::new(0)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NiceRestoreRecord {
    #[serde(default = "zero_tid")]
    pub tid: Tid,
    pub original_nice: i32,
    #[serde(default)]
    pub identity: Option<TaskRestoreIdentity>,
}

impl NiceRestoreRecord {
    pub fn new(identity: TaskRestoreIdentity, original_nice: i32) -> Self {
        Self {
            tid: identity.tid,
            original_nice,
            identity: Some(identity),
        }
    }

    pub fn tid(&self) -> u32 {
        self.identity
            .as_ref()
            .map_or(self.tid.as_u32(), |identity| identity.tid.as_u32())
    }

    pub fn restore_identity(&self) -> TaskRestoreIdentity {
        self.identity
            .clone()
            .unwrap_or_else(|| TaskRestoreIdentity::legacy(self.tid.as_u32()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UclampRestoreRecord {
    #[serde(default = "zero_tid")]
    pub tid: Tid,
    pub original_util_min: u32,
    pub original_util_max: u32,
    #[serde(default)]
    pub identity: Option<TaskRestoreIdentity>,
}

impl UclampRestoreRecord {
    pub fn new(
        identity: TaskRestoreIdentity,
        original_util_min: u32,
        original_util_max: u32,
    ) -> Self {
        Self {
            tid: identity.tid,
            original_util_min,
            original_util_max,
            identity: Some(identity),
        }
    }

    pub fn tid(&self) -> u32 {
        self.identity
            .as_ref()
            .map_or(self.tid.as_u32(), |identity| identity.tid.as_u32())
    }

    pub fn restore_identity(&self) -> TaskRestoreIdentity {
        self.identity
            .clone()
            .unwrap_or_else(|| TaskRestoreIdentity::legacy(self.tid.as_u32()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IrqAffinityRestoreRecord {
    pub irq: u32,
    pub device_hint: String,
    pub original_smp_affinity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IoPrioRestoreRecord {
    #[serde(default = "zero_tid")]
    pub tid: Tid,
    pub original_ioprio: i32,
    #[serde(default)]
    pub identity: Option<TaskRestoreIdentity>,
}

impl IoPrioRestoreRecord {
    pub fn new(identity: TaskRestoreIdentity, original_ioprio: i32) -> Self {
        Self {
            tid: identity.tid,
            original_ioprio,
            identity: Some(identity),
        }
    }

    pub fn tid(&self) -> u32 {
        self.identity
            .as_ref()
            .map_or(self.tid.as_u32(), |identity| identity.tid.as_u32())
    }

    pub fn restore_identity(&self) -> TaskRestoreIdentity {
        self.identity
            .clone()
            .unwrap_or_else(|| TaskRestoreIdentity::legacy(self.tid.as_u32()))
    }
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
    #[serde(default = "zero_tid")]
    pub tid: Tid,
    pub original_cgroup: PathBuf,
    #[serde(default)]
    pub identity: Option<TaskRestoreIdentity>,
}

impl CgroupRestoreRecord {
    pub fn new(identity: TaskRestoreIdentity, original_cgroup: PathBuf) -> Self {
        Self {
            tid: identity.tid,
            original_cgroup,
            identity: Some(identity),
        }
    }

    pub fn tid(&self) -> u32 {
        self.identity
            .as_ref()
            .map_or(self.tid.as_u32(), |identity| identity.tid.as_u32())
    }

    pub fn restore_identity(&self) -> TaskRestoreIdentity {
        self.identity
            .clone()
            .unwrap_or_else(|| TaskRestoreIdentity::legacy(self.tid.as_u32()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CgroupCpusetRestoreRecord {
    pub cgroup_path: PathBuf,
    pub original_cpuset_cpus: Option<String>,
    pub original_cpuset_mems: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::{ActionId, ActionPreflightReport, ActionWarning};

    #[test]
    fn action_preflight_report_preserves_success_warnings() {
        let report = ActionPreflightReport::from_preflight_result(
            ActionId::new("test-action"),
            Ok::<_, anyhow::Error>(vec![ActionWarning {
                message: "soft warning".to_owned(),
            }]),
        );

        assert_eq!(report.action_id.as_str(), "test-action");
        assert!(!report.is_blocked());
        assert_eq!(report.warnings.len(), 1);
        assert!(report.blocker_messages().is_empty());
    }

    #[test]
    fn action_preflight_report_converts_errors_to_blockers() {
        let report = ActionPreflightReport::from_preflight_result(
            ActionId::new("test-action"),
            Err::<Vec<ActionWarning>, _>(anyhow::anyhow!("policy rejected")),
        );

        assert!(report.is_blocked());
        assert!(report.warnings.is_empty());
        assert_eq!(report.blocker_messages(), "policy rejected");
    }
}
