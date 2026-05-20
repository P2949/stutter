use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{
    actions::SafetyClass,
    daemon::config::{
        DaemonCandidateConfidenceConfig, DaemonCgroupTargetsConfig, DaemonSystemWideAllowlistConfig,
    },
};

pub(crate) mod build;
pub(crate) mod capability;
pub(crate) mod context;
pub(crate) mod evaluate;
pub(crate) mod explain;
pub(crate) mod model;
pub(crate) mod rejection;
pub(crate) mod remote;

pub use build::build_daemon_policy;
pub use context::{DaemonPolicyBuildInput, DaemonPolicyContext, RemotePolicyContext};
pub use model::{
    ActionDescriptor, ActionEffectScope, ActionSource, DaemonMode, DaemonPolicyVerdict,
    PolicyIntent, RollbackRequirement,
};
pub use rejection::PolicyRejection;
pub use remote::RemoteApplyPolicy;

pub const HIGH_RISK_APPLY_IMPLEMENTED: bool = false;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DaemonPolicy {
    pub mode: DaemonMode,
    pub source: ActionSource,
    pub max_safety_class: SafetyClass,
    pub allowed_effect_scopes: BTreeSet<ActionEffectScope>,
    pub enabled_action_families: BTreeSet<String>,
    pub denied_action_families: BTreeSet<String>,
    pub cgroup_targets: DaemonCgroupTargetsConfig,
    pub system_wide_allowlist: DaemonSystemWideAllowlistConfig,
    pub rollback_required_before_apply: bool,
    pub allow_medium_risk_apply: bool,
    pub allow_system_wide_suggestions: bool,
    pub allow_system_wide_apply: bool,
    pub allow_high_risk: bool,
    pub allow_persistent_effects: bool,
    pub allow_cpu_power_on_battery: bool,
    pub allow_gpu_power_in_autotune: bool,
    pub allow_vm_knobs_in_autotune: bool,
    pub high_risk_dry_run: bool,
    pub min_confidence: f32,
    pub confidence: DaemonCandidateConfidenceConfig,
    pub remote_apply: RemoteApplyPolicy,
}

#[cfg(test)]
#[path = "policy_tests/mod.rs"]
mod tests;
