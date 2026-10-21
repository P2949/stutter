#[cfg(test)]
pub(crate) use std::path::PathBuf;

#[cfg(test)]
pub(crate) use crate::{
    actions::{ActionError, ActionPhase, RollbackToken, SafetyClass},
    daemon_policy::{
        ActionSource, DaemonMode, DaemonPolicy, DaemonPolicyBuildInput, DaemonPolicyContext,
        build_daemon_policy,
    },
};

mod audit;
mod execute;
mod model;
mod policy;
mod rollback;

pub use execute::{run_audited_action, run_audited_action_with_audit_path};
pub(crate) use execute::{
    run_audited_action_with_audit_path_and_hooks, run_audited_action_with_hooks,
};
pub(crate) use model::ActionHooks;
pub use model::{ActionRunPolicy, AuditedActionResult};

#[cfg(test)]
#[path = "runner_tests/mod.rs"]
mod tests;
