pub mod acceptance;
pub mod autotune;
pub mod capabilities;
pub mod config;
pub mod explain;
pub mod health;
pub mod lifecycle;
pub mod monitor;
pub mod overhead;
pub mod policy;
pub mod privilege;
pub mod runtime;
pub mod soak;
pub mod state;
pub mod state_builders;
pub mod store;
pub mod watchdog;

pub use acceptance::{
    DaemonAcceptanceReport, DaemonAcceptanceStep, run_fake_daemon_acceptance_suite,
};
pub use autotune::{AutotuneSubsystem, AutotuneSubsystemEvent};
pub use capabilities::{CapabilityProbe, CapabilityProbeRoot, DaemonCapabilities};
pub use config::{
    CgroupTargetRole, DaemonAutotuneConfig, DaemonCgroupTargetsConfig, DaemonConfig,
    DaemonHealthConfig, DaemonPreset, DaemonRemoteConfig, DaemonRetentionConfig,
    DaemonSafetyConfig, DaemonTargetConfig,
};
pub use explain::{
    DaemonPolicyExplanation, DaemonStatusExplanation, PolicyDecisionKind, PolicyExplainLine,
    PolicyExplanation, PolicyRuleEvaluation, policy_context_from_daemon_status,
    policy_context_from_daemon_status_at,
};
pub use health::{
    SystemHealthInputs, SystemHealthIssue, SystemHealthMonitor, SystemHealthProbeRoot,
    SystemHealthSnapshot, SystemHealthState, SystemHealthThresholds, evaluate_system_health,
};
pub use lifecycle::{
    DaemonLifecycleAction, DaemonLifecycleEvent, DaemonLifecycleInputs, DaemonLifecyclePolicy,
    DaemonLifecycleTransition, LifecycleClockSample, SuspendResumeDetector,
    evaluate_daemon_lifecycle_event,
};
pub use monitor::{MonitorShutdownSummary, MonitorSubsystem, MonitorSubsystemConfig};
pub use overhead::{
    DaemonOverheadBudget, DaemonOverheadIssue, DaemonOverheadMonitor, DaemonOverheadReport,
    DaemonOverheadSnapshot, evaluate_daemon_overhead,
};
pub use policy::{
    ActionDescriptor, ActionEffectScope, ActionSource, DaemonMode, DaemonPolicy,
    DaemonPolicyBuildInput, DaemonPolicyContext, DaemonPolicyVerdict, PolicyIntent,
    PolicyRejection, RemoteApplyPolicy, RemotePolicyContext, RollbackRequirement,
    build_daemon_policy,
};
pub use privilege::{
    PrivilegeCommandAllowlist, PrivilegeCommandRequest, PrivilegeDecision, PrivilegeProcessRole,
    PrivilegeTransport, PrivilegedOperation, privileged_operation_audit_event,
};
pub use runtime::{DaemonRuntime, DaemonRuntimeConfig, DaemonRuntimeEvent, DaemonTransition};
pub use soak::{
    DaemonSoakBudget, DaemonSoakConfig, DaemonSoakFailure, DaemonSoakMetrics, DaemonSoakProfile,
    DaemonSoakReport, run_fake_daemon_soak,
};
pub use state::{
    DAEMON_STATE_SCHEMA_VERSION, DaemonDecisionState, DaemonDegradedStatus, DaemonExperimentState,
    DaemonFaultState, DaemonPhase, DaemonProfileEnvironment, DaemonProfileMemory,
    DaemonProfilePartition, DaemonProfileValidation, DaemonRollbackState, DaemonState,
    DaemonStateSnapshotWriter, DaemonTargetState, DaemonWorkloadProfile,
    default_daemon_state_snapshot_path, load_daemon_state,
};
pub use state_builders::{
    StartupRecoveryDaemonStateInput, daemon_decision_state, daemon_state_for_agent_fault,
    daemon_state_for_record_start, daemon_state_for_startup_recovery_snapshot,
    daemon_state_from_startup_recovery, safety_class_for_rollback_token,
};
pub use store::DaemonStateStore;
pub use watchdog::{
    DaemonSelfHealingAction, DaemonWatchdogConfig, DaemonWatchdogInputs, DaemonWatchdogIssue,
    DaemonWatchdogReport, evaluate_daemon_watchdog,
};
