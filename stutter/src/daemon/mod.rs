pub mod autotune;
pub mod config;
pub mod explain;
pub mod monitor;
pub mod policy;
pub mod runtime;
pub mod state;

pub use autotune::{AutotuneSubsystem, AutotuneSubsystemEvent};
pub use config::{
    DaemonAutotuneConfig, DaemonConfig, DaemonRemoteConfig, DaemonRetentionConfig,
    DaemonSafetyConfig, DaemonTargetConfig,
};
pub use explain::{
    DaemonPolicyExplanation, PolicyDecisionKind, PolicyExplainLine, PolicyExplanation,
    PolicyRuleEvaluation,
};
pub use monitor::{MonitorShutdownSummary, MonitorSubsystem, MonitorSubsystemConfig};
pub use policy::{
    ActionDescriptor, ActionEffectScope, ActionSource, DaemonMode, DaemonPolicy,
    DaemonPolicyBuildInput, PolicyIntent, PolicyRejection, RemoteApplyPolicy, RemotePolicyContext,
    RollbackRequirement, build_daemon_policy,
};
pub use runtime::{DaemonRuntime, DaemonRuntimeConfig, DaemonRuntimeEvent, DaemonTransition};
pub use state::{
    DAEMON_STATE_SCHEMA_VERSION, DaemonDecisionState, DaemonDegradedStatus, DaemonExperimentState,
    DaemonFaultState, DaemonPhase, DaemonRollbackState, DaemonState, DaemonStateSnapshotWriter,
    DaemonTargetState, default_daemon_state_snapshot_path, load_daemon_state,
};
