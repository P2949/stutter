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

pub(crate) mod error;
pub(crate) mod factory;
pub(crate) mod model;
pub(crate) mod restore_identity;
pub(crate) mod restore_write;
pub(crate) mod rollback;
pub(crate) mod syscalls;
pub(crate) mod token;
pub(crate) mod traits;
pub(crate) mod transaction;

pub use cpu_power::{CpuPowerAction, CpuPowerPolicy};
pub use error::{
    ActionBoundaryError, ActionError, ActionFailure, ActionResult, ActionTimeout, ApplyResult,
    PartialApplyError, PhaseFailure, RollbackOutcome, ScopeLimitExceeded,
};
pub use factory::default_action_factory_registry;
pub use gpu_power::{GpuPowerAction, GpuPowerMode, GpuPowerPolicy};
pub use irq_affinity::{
    IrqAffinityAction, IrqAffinityEvidence, IrqAffinityPolicy, IrqAffinityRisk,
};
pub use model::{
    ActionId, ActionOutcome, ActionPhase, ActionPreflightReport, ActionState, ActionWarning,
    CgroupCpusetRestoreRecord, CgroupRestoreRecord, CpuPowerRestoreRecord, GpuPowerRestoreRecord,
    IoPrioRestoreRecord, IrqAffinityRestoreRecord, NiceRestoreRecord, SafetyClass, TaskIdentity,
    TaskRestoreIdentity, UclampRestoreRecord, VmKnobRestoreRecord,
};
pub use nice::{NiceAction, NicePolicy};
pub use restore_identity::{RestoreIdentityStatus, verify_task_identity};
pub use rollback::{
    RestoreAllInput, RestoreAllSummary, RollbackCandidate, RollbackHandler, RollbackPreview,
    RollbackRegistry, RollbackResult, default_rollback_registry,
};
pub use token::{RollbackToken, RollbackTokenKindError};
pub use traits::TuningAction;
pub use vm_knobs::{VmKnobAction, VmKnobChange, VmKnobMode, VmKnobPolicy};

#[cfg(test)]
mod tests {
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

    #[test]
    fn action_root_module_is_thin_facade() {
        let source = include_str!("mod.rs");

        for (kind, name) in [
            ("pub enum", "ActionError"),
            ("pub enum", "SafetyClass"),
            ("pub struct", "RollbackRegistry"),
            ("pub enum", "RollbackToken"),
            ("pub trait", "TuningAction"),
        ] {
            let needle = format!("{kind} {name}");
            assert!(
                !source.contains(&needle),
                "actions/mod.rs must re-export domain contracts from focused modules instead of defining {needle}"
            );
        }
    }

    #[test]
    fn action_root_has_no_broad_allow_suppressions() {
        let source = include_str!("mod.rs");
        let allow_needle = concat!("#[", "allow(");

        assert!(
            !source.contains(allow_needle),
            "actions/mod.rs should stay a narrow facade without broad allow suppressions"
        );
    }
}
