//! Action model and audited host mutation primitives.
//!
//! Owns:
//! - the shared action lifecycle types, safety classes, warnings, outcomes, rollback tokens,
//!   rollback registry, and `TuningAction` trait used by concrete action submodules.
//!
//! Does not own:
//! - daemon policy decisions, autotune planning, CLI parsing, live recording, or report rendering.
//!
//! Allowed dependencies:
//! - low-level system helpers and safety support such as affinity, audit, hwmon, irq inspection,
//!   process tree data, procfs helpers, profile restore records, system inventory, task classes,
//!   task expansion, topology, and daemon policy metadata needed to describe safety.
//!
//! Main entry points:
//! - `ActionPhase`, `ActionError`, `ActionId`, `SafetyClass`, `RollbackRegistry`,
//!   `RollbackToken`, `TuningAction`, `RollbackHandler`, and the concrete action submodules.
//!
//! Safety, mutation, and persistence invariants:
//! - every mutating action must support preflight/dry-run/apply/verify/rollback sequencing;
//! - rollback state must be represented as explicit restore records or rollback tokens;
//! - scope limits and daemon policy rejections must be surfaced as `ActionError` values;
//! - this module may describe and execute host mutations, but it must not silently persist
//!   undeclared state or bypass rollback accounting.

#[cfg(test)]
pub(crate) mod fake_action;

pub(crate) mod cgroup;
pub(crate) mod cpu_affinity;
pub(crate) mod cpu_power;
pub(crate) mod gpu_power;
pub(crate) mod nice;

pub(crate) mod ioprio;
pub(crate) mod irq_affinity;
pub(crate) mod uclamp;
pub(crate) mod vm_knobs;

pub(crate) mod runner;

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
    ScopeLimitExceeded {
        affected_tasks: usize,
        max_affected_tasks: usize,
    },
    Timeout {
        phase: ActionPhase,
        elapsed_ms: u128,
        timeout_ms: u128,
    },
    TimeoutRollbackCompleted {
        phase: ActionPhase,
        elapsed_ms: u128,
        timeout_ms: u128,
    },
    TimeoutRollbackFailure {
        phase: ActionPhase,
        elapsed_ms: u128,
        timeout_ms: u128,
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

    pub fn scope_limit_exceeded(affected_tasks: usize, max_affected_tasks: usize) -> Self {
        Self::ScopeLimitExceeded {
            affected_tasks,
            max_affected_tasks,
        }
    }

    pub fn timeout(phase: ActionPhase, elapsed_ms: u128, timeout_ms: u128) -> Self {
        Self::Timeout {
            phase,
            elapsed_ms,
            timeout_ms,
        }
    }

    pub fn timeout_rollback_completed(
        phase: ActionPhase,
        elapsed_ms: u128,
        timeout_ms: u128,
    ) -> Self {
        Self::TimeoutRollbackCompleted {
            phase,
            elapsed_ms,
            timeout_ms,
        }
    }

    pub fn timeout_rollback_failure(
        phase: ActionPhase,
        elapsed_ms: u128,
        timeout_ms: u128,
        rollback_error: impl std::fmt::Display,
    ) -> Self {
        Self::TimeoutRollbackFailure {
            phase,
            elapsed_ms,
            timeout_ms,
            rollback_error: rollback_error.to_string(),
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
            Self::ScopeLimitExceeded { .. } => ActionPhase::DryRun,
            Self::Timeout { phase, .. } | Self::TimeoutRollbackCompleted { phase, .. } => *phase,
            Self::TimeoutRollbackFailure { .. } => ActionPhase::EmergencyRollback,
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
            Self::ScopeLimitExceeded { .. } => "scope_limit_exceeded",
            Self::Timeout { .. } => "timeout",
            Self::TimeoutRollbackCompleted { .. } => "timeout_rollback_completed",
            Self::TimeoutRollbackFailure { .. } => "timeout_rollback_failure",
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
            Self::ScopeLimitExceeded {
                affected_tasks,
                max_affected_tasks,
            } => format!(
                "dry run affected {affected_tasks} task(s), exceeding scope limit {max_affected_tasks}"
            ),
            Self::Timeout {
                phase,
                elapsed_ms,
                timeout_ms,
            } => format!(
                "action timed out during {}: elapsed_ms={} timeout_ms={}",
                phase.as_str(),
                elapsed_ms,
                timeout_ms
            ),
            Self::TimeoutRollbackCompleted {
                phase,
                elapsed_ms,
                timeout_ms,
            } => format!(
                "action timed out during {}; rollback completed: elapsed_ms={} timeout_ms={}",
                phase.as_str(),
                elapsed_ms,
                timeout_ms
            ),
            Self::TimeoutRollbackFailure {
                phase,
                elapsed_ms,
                timeout_ms,
                rollback_error,
            } => format!(
                "action timed out during {}; emergency rollback failed: elapsed_ms={} timeout_ms={}; rollback error: {}",
                phase.as_str(),
                elapsed_ms,
                timeout_ms,
                rollback_error
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

#[allow(dead_code)] // TODO: replace emergency_restore direct dispatch with RollbackRegistry.
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

    #[allow(dead_code)] // TODO: replace emergency_restore direct dispatch with RollbackRegistry.
    pub fn discover_all(&self) -> anyhow::Result<Vec<RollbackCandidate>> {
        let mut candidates = Vec::new();
        for handler in &self.handlers {
            candidates.extend(handler.discover()?);
        }
        Ok(candidates)
    }

    #[allow(dead_code)] // TODO: replace emergency_restore direct dispatch with RollbackRegistry.
    pub fn preview_all(&self) -> anyhow::Result<Vec<RollbackPreview>> {
        let mut previews = Vec::new();
        for handler in &self.handlers {
            for candidate in handler.discover()? {
                previews.push(handler.dry_run(&candidate)?);
            }
        }
        Ok(previews)
    }

    #[allow(dead_code)] // TODO: replace emergency_restore direct dispatch with RollbackRegistry.
    pub fn restore_all(&self, input: RestoreAllInput) -> RestoreAllSummary {
        let mut summary = RestoreAllSummary::default();

        for handler in &self.handlers {
            let candidates = match handler.discover() {
                Ok(candidates) => candidates,
                Err(err) => {
                    log::warn!(
                        "rollback_discover_failed handler={} err={err:#}",
                        handler.id()
                    );
                    summary.errors += 1;
                    continue;
                }
            };

            for candidate in candidates {
                if input.dry_run {
                    match handler.dry_run(&candidate) {
                        Ok(preview) => {
                            summary.restored_total += preview.affected_tasks;
                        }
                        Err(err) => {
                            log::warn!(
                                "rollback_dry_run_failed handler={} path={} err={err:#}",
                                handler.id(),
                                candidate.restore_path.display()
                            );
                            summary.errors += 1;
                        }
                    }
                    continue;
                }

                match handler.restore(candidate) {
                    Ok(result) => {
                        summary.restored_total += result.restored;
                        summary.skipped_dead += result.skipped_dead;
                        summary.skipped_identity_mismatch += result.skipped_identity_mismatch;
                        summary.legacy_unverified += result.legacy_unverified;
                        summary.errors += result.errors;
                    }
                    Err(err) => {
                        log::warn!(
                            "rollback_restore_failed handler={} err={err:#}",
                            handler.id()
                        );
                        summary.errors += 1;
                    }
                }
            }
        }

        summary
    }
}

impl Default for RollbackRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(dead_code)] // TODO: replace emergency_restore direct dispatch with RollbackRegistry.
pub trait RollbackHandler {
    fn id(&self) -> &'static str;
    fn discover(&self) -> anyhow::Result<Vec<RollbackCandidate>>;
    fn dry_run(&self, candidate: &RollbackCandidate) -> anyhow::Result<RollbackPreview>;
    fn restore(&self, candidate: RollbackCandidate) -> anyhow::Result<RollbackResult>;
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // TODO: replace emergency_restore direct dispatch with RollbackRegistry.
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn action_child_modules_are_not_public_submodules() {
        let source = include_str!("mod.rs");

        let public_child_modules: Vec<&str> = source
            .lines()
            .map(str::trim_start)
            .filter(|line| line.starts_with("pub mod "))
            .collect();

        assert!(
            public_child_modules.is_empty(),
            "actions child modules must stay crate-private and be exposed intentionally through api::actions: {public_child_modules:?}"
        );
    }

    struct TestRollbackHandler {
        id: &'static str,
        candidates: Vec<PathBuf>,
        restore_result: RollbackResult,
        fail_discover: bool,
        fail_dry_run: bool,
        fail_restore: bool,
    }

    impl TestRollbackHandler {
        fn new(id: &'static str, candidates: Vec<&str>) -> Self {
            Self {
                id,
                candidates: candidates.into_iter().map(PathBuf::from).collect(),
                restore_result: RollbackResult {
                    handler_id: id,
                    restore_path: PathBuf::from("/tmp/restore"),
                    restored: 1,
                    skipped_dead: 0,
                    skipped_identity_mismatch: 0,
                    legacy_unverified: 0,
                    errors: 0,
                },
                fail_discover: false,
                fail_dry_run: false,
                fail_restore: false,
            }
        }

        fn with_restore_result(mut self, restore_result: RollbackResult) -> Self {
            self.restore_result = restore_result;
            self
        }

        fn with_discover_failure(mut self) -> Self {
            self.fail_discover = true;
            self
        }

        fn with_dry_run_failure(mut self) -> Self {
            self.fail_dry_run = true;
            self
        }

        fn with_restore_failure(mut self) -> Self {
            self.fail_restore = true;
            self
        }
    }

    impl RollbackHandler for TestRollbackHandler {
        fn id(&self) -> &'static str {
            self.id
        }

        fn discover(&self) -> anyhow::Result<Vec<RollbackCandidate>> {
            if self.fail_discover {
                anyhow::bail!("discover failed");
            }
            Ok(self
                .candidates
                .iter()
                .cloned()
                .map(|restore_path| RollbackCandidate {
                    handler_id: self.id,
                    restore_path,
                })
                .collect())
        }

        fn dry_run(&self, candidate: &RollbackCandidate) -> anyhow::Result<RollbackPreview> {
            if self.fail_dry_run {
                anyhow::bail!("dry run failed");
            }
            Ok(RollbackPreview {
                handler_id: self.id,
                restore_path: candidate.restore_path.clone(),
                affected_tasks: 2,
                message: "would restore test candidate".to_owned(),
            })
        }

        fn restore(&self, candidate: RollbackCandidate) -> anyhow::Result<RollbackResult> {
            if self.fail_restore {
                anyhow::bail!("restore failed");
            }
            let mut result = self.restore_result.clone();
            result.restore_path = candidate.restore_path;
            Ok(result)
        }
    }

    #[test]
    fn rollback_registry_discovers_and_previews_all_handlers() {
        let mut registry = RollbackRegistry::new();
        registry.register(TestRollbackHandler::new("affinity", vec!["/tmp/a"]));
        registry.register(TestRollbackHandler::new(
            "profile",
            vec!["/tmp/b", "/tmp/c"],
        ));

        let candidates = registry.discover_all().unwrap();
        let previews = registry.preview_all().unwrap();

        assert_eq!(candidates.len(), 3);
        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.handler_id)
                .collect::<Vec<_>>(),
            vec!["affinity", "profile", "profile"]
        );
        assert_eq!(previews.len(), 3);
        assert!(previews.iter().all(|preview| preview.affected_tasks == 2));
    }

    #[test]
    fn rollback_registry_restore_all_aggregates_partial_failures() {
        let mut registry = RollbackRegistry::new();
        registry.register(
            TestRollbackHandler::new("good", vec!["/tmp/good"]).with_restore_result(
                RollbackResult {
                    handler_id: "good",
                    restore_path: PathBuf::from("/tmp/good"),
                    restored: 3,
                    skipped_dead: 1,
                    skipped_identity_mismatch: 2,
                    legacy_unverified: 4,
                    errors: 5,
                },
            ),
        );
        registry.register(
            TestRollbackHandler::new("restore-error", vec!["/tmp/bad"]).with_restore_failure(),
        );
        registry.register(
            TestRollbackHandler::new("discover-error", vec!["/tmp/missing"])
                .with_discover_failure(),
        );

        let summary = registry.restore_all(RestoreAllInput { dry_run: false });

        assert_eq!(summary.restored_total, 3);
        assert_eq!(summary.skipped_dead, 1);
        assert_eq!(summary.skipped_identity_mismatch, 2);
        assert_eq!(summary.legacy_unverified, 4);
        assert_eq!(summary.errors, 7);
    }

    #[test]
    fn rollback_registry_dry_run_restore_all_counts_previewed_records() {
        let mut registry = RollbackRegistry::new();
        registry.register(TestRollbackHandler::new("good", vec!["/tmp/a", "/tmp/b"]));
        registry.register(
            TestRollbackHandler::new("dry-run-error", vec!["/tmp/c"]).with_dry_run_failure(),
        );

        let summary = registry.restore_all(RestoreAllInput { dry_run: true });

        assert_eq!(summary.restored_total, 4);
        assert_eq!(summary.errors, 1);
        assert_eq!(summary.skipped_dead, 0);
    }
}
