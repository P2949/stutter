//! Daemon policy, state, privilege, health, and lifecycle facade.
//!
//! Owns:
//! - daemon state models, policy verdicts, capability and health probes, lifecycle decisions,
//!   watchdog/overhead/soak checks, state persistence, and privileged operation mediation.
//!
//! Does not own:
//! - CLI parsing, command dispatch, direct remote request handling, report rendering, or direct
//!   action mutation outside daemon policy and privilege boundaries.
//!
//! Allowed dependencies:
//! - actions, autotune coordination, config, process tree data, recorder/session models, and
//!   system probes needed to evaluate safe daemon behavior.
//!
//! Main entry points:
//! - `DaemonRuntime`, `DaemonPolicy`, `build_daemon_policy`, `DaemonState`,
//!   `DaemonStateStore`, capability/health/lifecycle/watchdog types, and the public re-exports
//!   from this module.
//!
//! Safety, mutation, and persistence invariants:
//! - every daemon-authorized mutation must pass through `DaemonPolicy` and privilege checks;
//! - persistent state must use the daemon state schema/versioned store types;
//! - health, watchdog, and lifecycle transitions must degrade or reject unsafe operations rather
//!   than silently continuing;
//! - this facade should re-export subsystem contracts without embedding CLI-only behavior.

pub(crate) mod acceptance;
pub(crate) mod autotune;
pub(crate) mod capabilities;
pub(crate) mod config;
pub(crate) mod explain;
pub(crate) mod health;
pub(crate) mod lifecycle;
pub(crate) mod monitor;
pub(crate) mod overhead;
pub(crate) mod policy;
pub(crate) mod privilege;
pub(crate) mod runtime;
pub(crate) mod soak;
pub(crate) mod state;
pub(crate) mod state_builders;
pub(crate) mod store;
pub(crate) mod watchdog;

pub(crate) use acceptance::{DaemonAcceptanceReport, run_fake_daemon_acceptance_suite};
pub(crate) use capabilities::{CapabilityProbe, DaemonCapabilities};
pub(crate) use config::{CgroupTargetRole, DaemonCgroupTargetsConfig, DaemonConfig, DaemonPreset};
pub(crate) use explain::{
    DaemonPolicyExplanation, DaemonStatusExplanation, PolicyDecisionKind, PolicyExplanation,
    policy_context_from_daemon_status,
};
pub(crate) use health::{
    SystemHealthInputs, SystemHealthMonitor, SystemHealthProbeRoot, SystemHealthSnapshot,
    SystemHealthState, SystemHealthThresholds, evaluate_system_health,
};
pub(crate) use lifecycle::{
    DaemonLifecycleAction, DaemonLifecycleEvent, DaemonLifecycleInputs, DaemonLifecyclePolicy,
    DaemonLifecycleTransition, evaluate_daemon_lifecycle_event,
};
pub(crate) use overhead::{DaemonOverheadMonitor, DaemonOverheadReport};
pub(crate) use policy::{
    ActionDescriptor, ActionEffectScope, ActionSource, DaemonMode, DaemonPolicy,
    DaemonPolicyBuildInput, DaemonPolicyContext, PolicyIntent, PolicyRejection,
    RemotePolicyContext, RollbackRequirement, build_daemon_policy,
};
pub(crate) use privilege::{
    PrivilegeCommandAllowlist, PrivilegeCommandRequest, PrivilegeProcessRole, PrivilegeTransport,
    PrivilegedOperation,
};
pub(crate) use soak::{
    DaemonSoakBudget, DaemonSoakConfig, DaemonSoakProfile, DaemonSoakReport, run_fake_daemon_soak,
};
pub(crate) use state::{
    DAEMON_STATE_SCHEMA_VERSION, DaemonDecisionState, DaemonDegradedStatus, DaemonExperimentState,
    DaemonFaultState, DaemonPhase, DaemonProfileEnvironment, DaemonProfileValidation,
    DaemonRollbackState, DaemonState, DaemonStateSnapshotWriter, DaemonTargetState,
    DaemonWorkloadProfile, default_daemon_state_snapshot_path, load_daemon_state,
};
pub(crate) use state_builders::{
    StartupRecoveryDaemonStateInput, daemon_decision_state, daemon_state_for_agent_fault,
    daemon_state_for_record_start, daemon_state_for_startup_recovery_snapshot,
    daemon_state_from_startup_recovery, safety_class_for_rollback_token,
};
pub(crate) use store::DaemonStateStore;
pub(crate) use watchdog::{
    DaemonWatchdogConfig, DaemonWatchdogInputs, DaemonWatchdogReport, evaluate_daemon_watchdog,
};

#[cfg(test)]
mod tests {
    #[test]
    fn daemon_child_modules_are_not_public_submodules() {
        let source = include_str!("mod.rs");

        let public_child_modules: Vec<&str> = source
            .lines()
            .map(str::trim_start)
            .filter(|line| line.starts_with("pub mod "))
            .collect();

        assert!(
            public_child_modules.is_empty(),
            "daemon child modules must stay crate-private and be exposed intentionally through api::daemon: {public_child_modules:?}"
        );
    }
}
