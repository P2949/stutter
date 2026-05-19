use std::{collections::BTreeSet, fmt, str::FromStr};

use serde::{Deserialize, Serialize};

pub use crate::error::DaemonPolicyError;
use crate::{
    actions::{ActionId, SafetyClass},
    daemon::{
        capabilities::DaemonCapabilities,
        config::{DaemonCandidateConfidenceConfig, DaemonCgroupTargetsConfig, DaemonConfig},
        explain::{PolicyDecisionKind, PolicyExplanation, PolicyRuleEvaluation},
        health::SystemHealthSnapshot,
    },
    remote::AgentAutotuneLimits,
};

pub(crate) mod context;
pub(crate) mod evaluate;
pub(crate) mod explain;
pub(crate) mod model;

pub const HIGH_RISK_APPLY_IMPLEMENTED: bool = false;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DaemonMode {
    Observe,
    Suggest,
    ApplyLowRisk,
    ApplyMediumRisk,
    ApplyHighRisk,
}

impl DaemonMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Observe => "observe",
            Self::Suggest => "suggest",
            Self::ApplyLowRisk => "apply-low-risk",
            Self::ApplyMediumRisk => "apply-medium-risk",
            Self::ApplyHighRisk => "apply-high-risk",
        }
    }

    pub fn supports_apply(self) -> bool {
        matches!(
            self,
            Self::ApplyLowRisk | Self::ApplyMediumRisk | Self::ApplyHighRisk
        )
    }
}

impl fmt::Display for DaemonMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for DaemonMode {
    type Err = PolicyRejection;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "observe" => Ok(Self::Observe),
            "suggest" => Ok(Self::Suggest),
            "apply-low-risk" => Ok(Self::ApplyLowRisk),
            "apply-medium-risk" => Ok(Self::ApplyMediumRisk),
            "apply-high-risk" => Ok(Self::ApplyHighRisk),
            other => Err(PolicyRejection::UnsupportedMode {
                mode: other.to_owned(),
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionEffectScope {
    ObserveOnly,
    LocalProcess,
    LocalProcessTree,
    UserStateFile,
    Cgroup,
    Irq,
    Sysfs,
    CpuPower,
    GpuPower,
    VmKnob,
    SystemWide,
}

impl ActionEffectScope {
    fn is_low_risk_apply_scope(self) -> bool {
        matches!(self, Self::LocalProcess | Self::LocalProcessTree)
    }

    fn is_explicit_target_scope(self) -> bool {
        matches!(
            self,
            Self::LocalProcess | Self::LocalProcessTree | Self::Cgroup
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RollbackRequirement {
    NotRequiredForDryRun,
    RequiredBeforeApply,
    BestEffortOnly,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionSource {
    Cli,
    AutotuneRuntime,
    RemoteAgent,
    Tune,
    ApplyProfileWatch,
    Test,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActionDescriptor {
    pub action_id: ActionId,
    pub action_kind: String,
    pub safety_class: SafetyClass,
    pub effect_scope: ActionEffectScope,
    pub rollback: RollbackRequirement,
    pub persistent_effect: bool,
    pub touches_system_wide_state: bool,
    pub requires_explicit_target: bool,
    pub confidence: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonPolicyContext {
    pub data_quality_ok: bool,
    pub data_quality_reason_code: Option<String>,
    pub system_health_ok: bool,
    pub system_health_reason_code: Option<String>,
    pub workload_stable: bool,
    pub cooldown_active: bool,
    pub rollback_pending: bool,
    pub capabilities: Option<DaemonCapabilities>,
}

impl Default for DaemonPolicyContext {
    fn default() -> Self {
        Self {
            data_quality_ok: true,
            data_quality_reason_code: None,
            system_health_ok: true,
            system_health_reason_code: None,
            workload_stable: true,
            cooldown_active: false,
            rollback_pending: false,
            capabilities: None,
        }
    }
}

impl DaemonPolicyContext {
    pub fn with_system_health(mut self, health: &SystemHealthSnapshot) -> Self {
        self.system_health_ok = health.ok_for_apply;
        self.system_health_reason_code = health.reason_code.clone();
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteApplyPolicy {
    pub allow_remote_apply: bool,
    pub remote_apply_enabled: bool,
    pub require_loopback_bind: bool,
    pub require_auth: bool,
    pub max_remote_targets: usize,
    pub target_count: usize,
    pub target_count_allowed: bool,
    pub mode_supported_by_limits: bool,
    pub auth_configured: bool,
    pub request_authorized: bool,
    pub bind_is_loopback: bool,
}

impl Default for RemoteApplyPolicy {
    fn default() -> Self {
        Self {
            allow_remote_apply: false,
            remote_apply_enabled: false,
            require_loopback_bind: true,
            require_auth: true,
            max_remote_targets: 1,
            target_count: 0,
            target_count_allowed: true,
            mode_supported_by_limits: false,
            auth_configured: false,
            request_authorized: false,
            bind_is_loopback: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DaemonPolicy {
    pub mode: DaemonMode,
    pub source: ActionSource,
    pub max_safety_class: SafetyClass,
    pub allowed_effect_scopes: BTreeSet<ActionEffectScope>,
    pub enabled_action_families: BTreeSet<String>,
    pub denied_action_families: BTreeSet<String>,
    pub cgroup_targets: DaemonCgroupTargetsConfig,
    pub rollback_required_before_apply: bool,
    pub allow_medium_risk_apply: bool,
    pub allow_system_wide_suggestions: bool,
    pub allow_system_wide_apply: bool,
    pub allow_high_risk: bool,
    pub allow_persistent_effects: bool,
    pub allow_cpu_power_on_battery: bool,
    pub min_confidence: f32,
    pub confidence: DaemonCandidateConfidenceConfig,
    pub remote_apply: RemoteApplyPolicy,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyIntent {
    Observe,
    Suggest,
    DryRun,
    Apply,
    Verify,
    Rollback,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DaemonPolicyVerdict {
    Allow,
    Reject,
    Delay,
    RequireObserveOnly,
    RequireManualConfirmation,
}

impl DaemonPolicyVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Reject => "reject",
            Self::Delay => "delay",
            Self::RequireObserveOnly => "require_observe_only",
            Self::RequireManualConfirmation => "require_manual_confirmation",
        }
    }

    pub fn is_allowed(self) -> bool {
        matches!(self, Self::Allow)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PolicyRejection {
    UnsupportedMode {
        mode: String,
    },
    IntentNotAllowed {
        mode: DaemonMode,
        intent: PolicyIntent,
    },
    SafetyClassTooHigh {
        mode: DaemonMode,
        safety_class: SafetyClass,
    },
    EffectScopeNotAllowed {
        mode: DaemonMode,
        effect_scope: ActionEffectScope,
    },
    RollbackRequired {
        rollback: RollbackRequirement,
    },
    RollbackUnavailable,
    SystemWideActionBlocked,
    PersistentEffectBlocked,
    ActionFamilyNotEnabled {
        action_kind: String,
    },
    ActionFamilyDenied {
        action_kind: String,
    },
    CapabilityUnavailable {
        action_kind: String,
        feature: &'static str,
    },
    ExplicitTargetRequired,
    HighRiskRequiresExplicitOptIn,
    HighRiskApplyNotImplemented,
    MediumRiskApplyRequiresExplicitUnlock,
    ConfidenceTooLow {
        confidence: f32,
        min_confidence: f32,
    },
    DataQualityBlocked {
        reason_code: String,
    },
    SystemHealthBlocked {
        reason_code: String,
    },
    WorkloadUnstable,
    CooldownActive,
    RollbackPending,
    RemoteApplyDisabled,
    RemoteModeNotAllowed {
        mode: DaemonMode,
    },
    RemoteApplyRequiresConfiguredAuth,
    RemoteApplyRequiresAuthorizedRequest,
    RemoteApplyRequiresLoopbackBind,
    RemoteTargetCountTooHigh {
        target_count: usize,
        max_targets: usize,
    },
}

impl fmt::Display for PolicyRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedMode { mode } => {
                write!(f, "unsupported daemon mode: {mode}")
            }
            Self::IntentNotAllowed { mode, intent } => {
                write!(f, "intent {intent:?} is not allowed in daemon mode {mode}")
            }
            Self::SafetyClassTooHigh { mode, safety_class } => {
                write!(
                    f,
                    "safety class {safety_class:?} is not allowed in daemon mode {mode}"
                )
            }
            Self::EffectScopeNotAllowed { mode, effect_scope } => {
                write!(
                    f,
                    "effect scope {effect_scope:?} is not allowed in daemon mode {mode}"
                )
            }
            Self::RollbackRequired { rollback } => {
                write!(
                    f,
                    "apply requires rollback to be available before apply; got {rollback:?}"
                )
            }
            Self::RollbackUnavailable => {
                f.write_str("apply is rejected because rollback is unavailable")
            }
            Self::SystemWideActionBlocked => {
                f.write_str("system-wide action is blocked by daemon policy")
            }
            Self::PersistentEffectBlocked => {
                f.write_str("persistent effect is blocked by daemon policy")
            }
            Self::ActionFamilyNotEnabled { action_kind } => {
                write!(
                    f,
                    "action family is not enabled by daemon preset: {action_kind}"
                )
            }
            Self::ActionFamilyDenied { action_kind } => {
                write!(f, "action family is denied by daemon policy: {action_kind}")
            }
            Self::CapabilityUnavailable {
                action_kind,
                feature,
            } => {
                write!(
                    f,
                    "action {action_kind} requires unavailable daemon capability: {feature}"
                )
            }
            Self::ExplicitTargetRequired => f.write_str("apply requires an explicit target scope"),
            Self::HighRiskRequiresExplicitOptIn => {
                f.write_str("high-risk apply requires explicit high-risk opt-in")
            }
            Self::HighRiskApplyNotImplemented => f.write_str("high-risk apply is not implemented"),
            Self::MediumRiskApplyRequiresExplicitUnlock => {
                f.write_str("apply-medium-risk requires explicit medium-risk unlock")
            }
            Self::ConfidenceTooLow {
                confidence,
                min_confidence,
            } => {
                write!(
                    f,
                    "action confidence {confidence:.3} is below required minimum {min_confidence:.3}"
                )
            }
            Self::DataQualityBlocked { reason_code } => {
                write!(f, "data quality blocks apply: {reason_code}")
            }
            Self::SystemHealthBlocked { reason_code } => {
                write!(f, "system health blocks apply: {reason_code}")
            }
            Self::WorkloadUnstable => {
                f.write_str("apply is rejected because workload identity is unstable")
            }
            Self::CooldownActive => f.write_str("apply is rejected because cooldown is active"),
            Self::RollbackPending => {
                f.write_str("apply is rejected because a previous rollback is pending")
            }
            Self::RemoteApplyDisabled => {
                f.write_str("remote apply is disabled by daemon configuration")
            }
            Self::RemoteModeNotAllowed { mode } => {
                write!(
                    f,
                    "remote autotune mode {mode} is not allowed by configured remote limits"
                )
            }
            Self::RemoteApplyRequiresConfiguredAuth => {
                f.write_str("remote autotune apply mode requires a configured bearer token")
            }
            Self::RemoteApplyRequiresAuthorizedRequest => {
                f.write_str("remote autotune apply mode requires a valid bearer token")
            }
            Self::RemoteApplyRequiresLoopbackBind => {
                f.write_str("remote autotune apply mode is only allowed on loopback binds")
            }
            Self::RemoteTargetCountTooHigh {
                target_count,
                max_targets,
            } => {
                write!(
                    f,
                    "autotune target count {target_count} exceeds max_targets {max_targets}"
                )
            }
        }
    }
}

impl std::error::Error for PolicyRejection {}

impl PolicyRejection {
    pub fn verdict(&self) -> DaemonPolicyVerdict {
        match self {
            Self::CooldownActive => DaemonPolicyVerdict::Delay,
            Self::DataQualityBlocked { .. }
            | Self::SystemHealthBlocked { .. }
            | Self::WorkloadUnstable
            | Self::ConfidenceTooLow { .. } => DaemonPolicyVerdict::RequireObserveOnly,
            Self::HighRiskRequiresExplicitOptIn
            | Self::HighRiskApplyNotImplemented
            | Self::SystemWideActionBlocked
            | Self::PersistentEffectBlocked
            | Self::SafetyClassTooHigh {
                safety_class: SafetyClass::HighRisk,
                ..
            } => DaemonPolicyVerdict::RequireManualConfirmation,
            Self::UnsupportedMode { .. }
            | Self::IntentNotAllowed { .. }
            | Self::SafetyClassTooHigh { .. }
            | Self::EffectScopeNotAllowed { .. }
            | Self::RollbackRequired { .. }
            | Self::RollbackUnavailable
            | Self::ActionFamilyNotEnabled { .. }
            | Self::ActionFamilyDenied { .. }
            | Self::CapabilityUnavailable { .. }
            | Self::ExplicitTargetRequired
            | Self::MediumRiskApplyRequiresExplicitUnlock
            | Self::RollbackPending
            | Self::RemoteApplyDisabled
            | Self::RemoteModeNotAllowed { .. }
            | Self::RemoteApplyRequiresConfiguredAuth
            | Self::RemoteApplyRequiresAuthorizedRequest
            | Self::RemoteApplyRequiresLoopbackBind
            | Self::RemoteTargetCountTooHigh { .. } => DaemonPolicyVerdict::Reject,
        }
    }

    pub fn reason_code(&self) -> &'static str {
        match self {
            Self::UnsupportedMode { .. } => "unsupported_mode",
            Self::IntentNotAllowed { .. } => "intent_not_allowed",
            Self::SafetyClassTooHigh { .. } => "safety_class_too_high",
            Self::EffectScopeNotAllowed { .. } => "effect_scope_not_allowed",
            Self::RollbackRequired { .. } => "rollback_required",
            Self::RollbackUnavailable => "rollback_unavailable",
            Self::SystemWideActionBlocked => "system_wide_action_blocked",
            Self::PersistentEffectBlocked => "persistent_effect_blocked",
            Self::ActionFamilyNotEnabled { .. } => "action_family_not_enabled",
            Self::ActionFamilyDenied { .. } => "action_family_denied",
            Self::CapabilityUnavailable { .. } => "capability_unavailable",
            Self::ExplicitTargetRequired => "explicit_target_required",
            Self::HighRiskRequiresExplicitOptIn => "high_risk_requires_explicit_opt_in",
            Self::HighRiskApplyNotImplemented => "high_risk_apply_not_implemented",
            Self::MediumRiskApplyRequiresExplicitUnlock => {
                "medium_risk_apply_requires_explicit_unlock"
            }
            Self::ConfidenceTooLow { .. } => "confidence_too_low",
            Self::DataQualityBlocked { .. } => "data_quality_blocked",
            Self::SystemHealthBlocked { .. } => "system_health_blocked",
            Self::WorkloadUnstable => "workload_unstable",
            Self::CooldownActive => "cooldown_active",
            Self::RollbackPending => "rollback_pending",
            Self::RemoteApplyDisabled => "remote_apply_disabled",
            Self::RemoteModeNotAllowed { .. } => "remote_mode_not_allowed",
            Self::RemoteApplyRequiresConfiguredAuth => "remote_apply_requires_configured_auth",
            Self::RemoteApplyRequiresAuthorizedRequest => {
                "remote_apply_requires_authorized_request"
            }
            Self::RemoteApplyRequiresLoopbackBind => "remote_apply_requires_loopback_bind",
            Self::RemoteTargetCountTooHigh { .. } => "remote_target_count_too_high",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemotePolicyContext {
    pub bind_is_loopback: bool,
    pub auth_configured: bool,
    pub request_authorized: bool,
    pub limits: AgentAutotuneLimits,
}

#[derive(Clone, Debug)]
pub struct DaemonPolicyBuildInput<'a> {
    pub config: &'a DaemonConfig,
    pub remote_context: Option<RemotePolicyContext>,
}

pub fn build_daemon_policy(input: DaemonPolicyBuildInput<'_>) -> DaemonPolicy {
    let config = input.config;
    let remote_context = input.remote_context.as_ref();
    let mode_max_safety_class = max_safety_class_for_mode(config.mode);
    let mut max_safety_class = mode_max_safety_class.clone();

    if let Some(context) = remote_context
        && context.limits.max_safety_class < max_safety_class
    {
        max_safety_class = context.limits.max_safety_class.clone();
    }

    let min_confidence = min_confidence_for_config(config);
    let confidence = confidence_thresholds_for_config(config, min_confidence);

    let mut allow_high_risk = HIGH_RISK_APPLY_IMPLEMENTED
        && config.mode == DaemonMode::ApplyHighRisk
        && config.safety.allow_high_risk;
    let allow_medium_risk_apply =
        config.mode == DaemonMode::ApplyMediumRisk && config.autotune.allow_medium_risk_apply;
    let mut allow_system_wide_suggestions =
        config.mode != DaemonMode::Observe && config.safety.allow_system_wide_suggestions;
    let mut allow_system_wide_apply = HIGH_RISK_APPLY_IMPLEMENTED
        && config.mode == DaemonMode::ApplyHighRisk
        && config.safety.allow_high_risk
        && config.safety.allow_system_wide_apply;

    if let Some(context) = remote_context {
        allow_system_wide_suggestions &= context.limits.allow_system_wide_suggestions;
        allow_system_wide_apply &= context.limits.allow_system_wide_apply;
        allow_high_risk &= context.limits.allow_high_risk;
    }

    DaemonPolicy {
        mode: config.mode,
        source: config.source,
        max_safety_class,
        allowed_effect_scopes: allowed_effect_scopes_for_mode(config.mode),
        enabled_action_families: config.safety.enabled_action_families.clone(),
        denied_action_families: config.safety.denied_action_families.clone(),
        cgroup_targets: config.safety.cgroup_targets.clone(),
        rollback_required_before_apply: config.mode.supports_apply(),
        allow_medium_risk_apply,
        allow_system_wide_suggestions,
        allow_system_wide_apply,
        allow_high_risk,
        allow_persistent_effects: config.safety.allow_persistent_effects,
        allow_cpu_power_on_battery: config.autotune.allow_cpu_power_on_battery,
        min_confidence,
        confidence,
        remote_apply: remote_apply_policy_for_config(config, remote_context),
    }
}

fn max_safety_class_for_mode(mode: DaemonMode) -> SafetyClass {
    match mode {
        DaemonMode::Observe => SafetyClass::ObserveOnly,
        DaemonMode::Suggest => SafetyClass::HighRisk,
        DaemonMode::ApplyLowRisk => SafetyClass::ReversibleLowRisk,
        DaemonMode::ApplyMediumRisk => SafetyClass::ReversibleMediumRisk,
        DaemonMode::ApplyHighRisk => SafetyClass::HighRisk,
    }
}

fn allowed_effect_scopes_for_mode(mode: DaemonMode) -> BTreeSet<ActionEffectScope> {
    match mode {
        DaemonMode::ApplyLowRisk => BTreeSet::from([
            ActionEffectScope::LocalProcess,
            ActionEffectScope::LocalProcessTree,
        ]),
        DaemonMode::ApplyMediumRisk => BTreeSet::from([
            ActionEffectScope::LocalProcess,
            ActionEffectScope::LocalProcessTree,
            ActionEffectScope::Cgroup,
        ]),
        DaemonMode::Observe | DaemonMode::Suggest | DaemonMode::ApplyHighRisk => BTreeSet::from([
            ActionEffectScope::ObserveOnly,
            ActionEffectScope::LocalProcess,
            ActionEffectScope::LocalProcessTree,
            ActionEffectScope::UserStateFile,
            ActionEffectScope::Cgroup,
            ActionEffectScope::Irq,
            ActionEffectScope::Sysfs,
            ActionEffectScope::CpuPower,
            ActionEffectScope::GpuPower,
            ActionEffectScope::VmKnob,
            ActionEffectScope::SystemWide,
        ]),
    }
}

fn min_confidence_for_config(config: &DaemonConfig) -> f32 {
    if config.safety.min_confidence > 0.0 {
        config.safety.min_confidence
    } else if config.mode.supports_apply() {
        crate::autotune::DEFAULT_MIN_FOCUS_CONFIDENCE
    } else {
        0.0
    }
}

fn confidence_thresholds_for_config(
    config: &DaemonConfig,
    min_confidence: f32,
) -> DaemonCandidateConfidenceConfig {
    let configured = config.autotune.confidence;

    DaemonCandidateConfidenceConfig {
        min_suggest_confidence: configured.min_suggest_confidence,
        min_apply_low_risk_confidence: configured.min_apply_low_risk_confidence.max(min_confidence),
        min_apply_medium_risk_confidence: configured
            .min_apply_medium_risk_confidence
            .max(min_confidence)
            .max(0.85),
        min_high_risk_suggestion_confidence: configured.min_high_risk_suggestion_confidence,
    }
}

fn remote_apply_policy_for_config(
    config: &DaemonConfig,
    remote_context: Option<&RemotePolicyContext>,
) -> RemoteApplyPolicy {
    let Some(context) = remote_context else {
        return RemoteApplyPolicy::default();
    };

    let mode_supported_by_limits = remote_mode_supported_by_context(config.mode, context);
    let auth_allowed = !config.remote.require_auth_for_apply
        || (context.auth_configured && context.request_authorized);
    let bind_allowed = config.remote.allow_non_loopback_apply || context.bind_is_loopback;
    let target_count = remote_target_count_for_config(config);
    let target_count_allowed = target_count <= context.limits.max_targets;

    RemoteApplyPolicy {
        allow_remote_apply: config.remote.allow_remote_apply
            && config.mode.supports_apply()
            && mode_supported_by_limits
            && auth_allowed
            && bind_allowed
            && target_count_allowed,
        remote_apply_enabled: config.remote.allow_remote_apply,
        require_loopback_bind: config.mode.supports_apply()
            && !config.remote.allow_non_loopback_apply,
        require_auth: config.mode.supports_apply() && config.remote.require_auth_for_apply,
        max_remote_targets: context.limits.max_targets,
        target_count,
        target_count_allowed,
        mode_supported_by_limits,
        auth_configured: context.auth_configured,
        request_authorized: context.request_authorized,
        bind_is_loopback: context.bind_is_loopback,
    }
}

fn remote_mode_supported_by_context(mode: DaemonMode, context: &RemotePolicyContext) -> bool {
    let mode_max_safety_class = max_safety_class_for_mode(mode);

    mode <= context.limits.max_mode
        && mode_max_safety_class <= context.limits.max_safety_class
        && (mode != DaemonMode::ApplyHighRisk
            || (HIGH_RISK_APPLY_IMPLEMENTED && context.limits.allow_high_risk))
}

fn remote_target_count_for_config(config: &DaemonConfig) -> usize {
    config.target.target_pids.len()
        + config.target.tree_pids.len()
        + usize::from(config.target.watch_process.is_some())
}

fn action_kind_matches_family(action_kind: &str, family: &str) -> bool {
    action_kind == family
        || action_kind.strip_prefix(family).is_some_and(|suffix| {
            matches!(
                suffix.as_bytes().first(),
                Some(b':') | Some(b'-') | Some(b'_')
            )
        })
}

fn unavailable_capability_for_action(
    action_kind: &str,
    capabilities: &DaemonCapabilities,
) -> Option<&'static str> {
    if action_kind_matches_family(action_kind, "ionice") && !capabilities.ionice_available {
        Some("ionice")
    } else if action_kind_matches_family(action_kind, "uclamp") && !capabilities.uclamp_available {
        Some("uclamp")
    } else if (action_kind_matches_family(action_kind, "irq")
        || action_kind_matches_family(action_kind, "irq_affinity"))
        && !capabilities.irq_affinity_available
    {
        Some("irq_affinity")
    } else if (action_kind_matches_family(action_kind, "cgroup")
        || action_kind_matches_family(action_kind, "cpuset"))
        && !capabilities.cgroup_v2_available
    {
        Some("cgroup_v2")
    } else if (action_kind_matches_family(action_kind, "gpu")
        || action_kind_matches_family(action_kind, "gpu_power"))
        && !capabilities.gpu_sysfs_available
    {
        Some("gpu_sysfs")
    } else if action_kind_matches_family(action_kind, "scx") && !capabilities.sched_ext_available {
        Some("sched_ext")
    } else if (action_kind_matches_family(action_kind, "cpu_perf")
        || action_kind_matches_family(action_kind, "perf"))
        && !capabilities.perf_permissions_likely
    {
        Some("perf_permissions")
    } else {
        None
    }
}

fn config_for_constructor(
    mode: DaemonMode,
    source: ActionSource,
    allow_high_risk: bool,
) -> DaemonConfig {
    let mut config = DaemonConfig {
        mode,
        source,
        ..DaemonConfig::default()
    };
    config.safety.max_safety_class = max_safety_class_for_mode(mode);
    config.safety.allow_high_risk = allow_high_risk;
    config.autotune.allow_medium_risk_apply = mode == DaemonMode::ApplyMediumRisk;
    config.safety.min_confidence = min_confidence_for_config(&config);
    config
}

impl DaemonPolicy {
    pub fn provider_confidence_threshold(&self, descriptor: &ActionDescriptor) -> f32 {
        match self.mode {
            DaemonMode::Observe => 1.0,
            DaemonMode::Suggest if descriptor.safety_class == SafetyClass::HighRisk => {
                self.confidence.min_high_risk_suggestion_confidence
            }
            DaemonMode::Suggest => self.confidence.min_suggest_confidence,
            DaemonMode::ApplyLowRisk => self.confidence.min_apply_low_risk_confidence,
            DaemonMode::ApplyMediumRisk => self.confidence.min_apply_medium_risk_confidence,
            DaemonMode::ApplyHighRisk => self.confidence.min_high_risk_suggestion_confidence,
        }
    }
}

fn record_policy_rule(
    evaluated_rules: &mut Vec<PolicyRuleEvaluation>,
    first_rejection: &mut Option<PolicyRejection>,
    rule: &'static str,
    passed: bool,
    reason: String,
    rejection: Option<PolicyRejection>,
) {
    evaluated_rules.push(PolicyRuleEvaluation {
        rule,
        passed,
        reason,
    });

    if !passed && first_rejection.is_none() {
        *first_rejection = rejection;
    }
}

fn record_remote_target_count_rule(
    policy: &DaemonPolicy,
    evaluated_rules: &mut Vec<PolicyRuleEvaluation>,
    first_rejection: &mut Option<PolicyRejection>,
) {
    let remote = &policy.remote_apply;

    record_policy_rule(
        evaluated_rules,
        first_rejection,
        "remote_target_count",
        remote.target_count_allowed,
        if remote.target_count_allowed {
            format!(
                "remote target count {} is within max_targets {}",
                remote.target_count, remote.max_remote_targets
            )
        } else {
            format!(
                "remote target count {} exceeds max_targets {}",
                remote.target_count, remote.max_remote_targets
            )
        },
        (!remote.target_count_allowed).then_some(PolicyRejection::RemoteTargetCountTooHigh {
            target_count: remote.target_count,
            max_targets: remote.max_remote_targets,
        }),
    );
}

fn record_remote_apply_rules(
    policy: &DaemonPolicy,
    evaluated_rules: &mut Vec<PolicyRuleEvaluation>,
    first_rejection: &mut Option<PolicyRejection>,
) {
    let remote = &policy.remote_apply;

    record_policy_rule(
        evaluated_rules,
        first_rejection,
        "remote_apply_enabled",
        remote.remote_apply_enabled,
        if remote.remote_apply_enabled {
            "remote apply is enabled by daemon configuration".to_owned()
        } else {
            "remote apply is disabled by daemon configuration".to_owned()
        },
        (!remote.remote_apply_enabled).then_some(PolicyRejection::RemoteApplyDisabled),
    );

    if remote.require_auth {
        record_policy_rule(
            evaluated_rules,
            first_rejection,
            "remote_auth_configured",
            remote.auth_configured,
            if remote.auth_configured {
                "remote apply has configured bearer-token authentication".to_owned()
            } else {
                "remote apply requires configured bearer-token authentication".to_owned()
            },
            (!remote.auth_configured).then_some(PolicyRejection::RemoteApplyRequiresConfiguredAuth),
        );

        record_policy_rule(
            evaluated_rules,
            first_rejection,
            "remote_request_authorized",
            remote.request_authorized,
            if remote.request_authorized {
                "remote apply request is authorized".to_owned()
            } else {
                "remote apply request is not authorized".to_owned()
            },
            (!remote.request_authorized)
                .then_some(PolicyRejection::RemoteApplyRequiresAuthorizedRequest),
        );
    }

    if remote.require_loopback_bind {
        record_policy_rule(
            evaluated_rules,
            first_rejection,
            "remote_loopback_bind",
            remote.bind_is_loopback,
            if remote.bind_is_loopback {
                "remote apply bind address is loopback".to_owned()
            } else {
                "remote apply bind address is not loopback".to_owned()
            },
            (!remote.bind_is_loopback).then_some(PolicyRejection::RemoteApplyRequiresLoopbackBind),
        );
    }

    record_policy_rule(
        evaluated_rules,
        first_rejection,
        "remote_mode_supported",
        remote.mode_supported_by_limits,
        if remote.mode_supported_by_limits {
            format!(
                "remote mode {} is within configured remote limits",
                policy.mode
            )
        } else {
            format!(
                "remote mode {} exceeds configured remote limits",
                policy.mode
            )
        },
        (!remote.mode_supported_by_limits)
            .then_some(PolicyRejection::RemoteModeNotAllowed { mode: policy.mode }),
    );

    record_remote_target_count_rule(policy, evaluated_rules, first_rejection);
}

impl DaemonPolicy {
    pub fn observe(source: ActionSource) -> Self {
        let config = config_for_constructor(DaemonMode::Observe, source, false);
        build_daemon_policy(DaemonPolicyBuildInput {
            config: &config,
            remote_context: None,
        })
    }

    pub fn suggest(source: ActionSource) -> Self {
        let config = config_for_constructor(DaemonMode::Suggest, source, false);
        build_daemon_policy(DaemonPolicyBuildInput {
            config: &config,
            remote_context: None,
        })
    }

    pub fn apply_low_risk(source: ActionSource) -> Self {
        let config = config_for_constructor(DaemonMode::ApplyLowRisk, source, false);
        build_daemon_policy(DaemonPolicyBuildInput {
            config: &config,
            remote_context: None,
        })
    }

    pub fn apply_medium_risk(source: ActionSource) -> Self {
        let config = config_for_constructor(DaemonMode::ApplyMediumRisk, source, false);
        build_daemon_policy(DaemonPolicyBuildInput {
            config: &config,
            remote_context: None,
        })
    }

    pub fn apply_high_risk_explicit(source: ActionSource) -> Self {
        let config = config_for_constructor(DaemonMode::ApplyHighRisk, source, true);
        build_daemon_policy(DaemonPolicyBuildInput {
            config: &config,
            remote_context: None,
        })
    }

    pub fn explain_action_with_context(
        &self,
        intent: PolicyIntent,
        descriptor: &ActionDescriptor,
        context: &DaemonPolicyContext,
    ) -> PolicyExplanation {
        let mut evaluated_rules = Vec::new();
        let mut first_rejection = None;
        self.record_context_rules(
            &intent,
            descriptor,
            context,
            &mut evaluated_rules,
            &mut first_rejection,
        );

        if first_rejection.is_some() {
            return self.finish_policy_explanation(
                intent,
                descriptor,
                evaluated_rules,
                first_rejection,
            );
        }

        let mut explanation = self.explain_action(intent, descriptor);
        evaluated_rules.append(&mut explanation.evaluated_rules);
        explanation.evaluated_rules = evaluated_rules;
        explanation
    }

    pub fn explain_action(
        &self,
        intent: PolicyIntent,
        descriptor: &ActionDescriptor,
    ) -> PolicyExplanation {
        let mut evaluated_rules = Vec::new();
        let mut first_rejection = None;

        if self.source == ActionSource::RemoteAgent
            && !self.mode.supports_apply()
            && !self.remote_apply.target_count_allowed
        {
            record_remote_target_count_rule(self, &mut evaluated_rules, &mut first_rejection);
            return self.finish_policy_explanation(
                intent,
                descriptor,
                evaluated_rules,
                first_rejection,
            );
        }

        match intent {
            PolicyIntent::Observe
            | PolicyIntent::DryRun
            | PolicyIntent::Verify
            | PolicyIntent::Rollback => {
                record_policy_rule(
                    &mut evaluated_rules,
                    &mut first_rejection,
                    "intent_allowed",
                    true,
                    format!("intent {intent:?} is allowed without apply authorization"),
                    None,
                );
                return self.finish_policy_explanation(
                    intent,
                    descriptor,
                    evaluated_rules,
                    first_rejection,
                );
            }
            PolicyIntent::Suggest => {
                let passed = self.mode != DaemonMode::Observe;
                record_policy_rule(
                    &mut evaluated_rules,
                    &mut first_rejection,
                    "intent_allowed",
                    passed,
                    if passed {
                        format!("suggest intent is allowed in daemon mode {}", self.mode)
                    } else {
                        format!("suggest intent is not allowed in daemon mode {}", self.mode)
                    },
                    (!passed).then_some(PolicyRejection::IntentNotAllowed {
                        mode: self.mode,
                        intent: intent.clone(),
                    }),
                );

                let passed =
                    !descriptor.touches_system_wide_state || self.allow_system_wide_suggestions;
                record_policy_rule(
                    &mut evaluated_rules,
                    &mut first_rejection,
                    "system_wide_suggestion_permission",
                    passed,
                    if passed {
                        "system-wide suggestion permission is satisfied".to_owned()
                    } else {
                        "system-wide suggestion is blocked by daemon policy".to_owned()
                    },
                    (!passed).then_some(PolicyRejection::SystemWideActionBlocked),
                );

                return self.finish_policy_explanation(
                    intent,
                    descriptor,
                    evaluated_rules,
                    first_rejection,
                );
            }
            PolicyIntent::Apply => {
                let passed = self.mode.supports_apply();
                record_policy_rule(
                    &mut evaluated_rules,
                    &mut first_rejection,
                    "intent_allowed",
                    passed,
                    if passed {
                        format!("apply intent is allowed in daemon mode {}", self.mode)
                    } else {
                        format!("apply intent is not allowed in daemon mode {}", self.mode)
                    },
                    (!passed).then_some(PolicyRejection::IntentNotAllowed {
                        mode: self.mode,
                        intent: intent.clone(),
                    }),
                );

                if !passed {
                    return self.finish_policy_explanation(
                        intent,
                        descriptor,
                        evaluated_rules,
                        first_rejection,
                    );
                }

                if self.mode == DaemonMode::ApplyMediumRisk {
                    record_policy_rule(
                        &mut evaluated_rules,
                        &mut first_rejection,
                        "medium_risk_apply_unlock",
                        self.allow_medium_risk_apply,
                        if self.allow_medium_risk_apply {
                            "apply-medium-risk explicit unlock is present".to_owned()
                        } else {
                            "apply-medium-risk requires explicit medium-risk unlock".to_owned()
                        },
                        (!self.allow_medium_risk_apply)
                            .then_some(PolicyRejection::MediumRiskApplyRequiresExplicitUnlock),
                    );
                }

                if self.mode == DaemonMode::ApplyHighRisk {
                    record_policy_rule(
                        &mut evaluated_rules,
                        &mut first_rejection,
                        "high_risk_apply_support",
                        HIGH_RISK_APPLY_IMPLEMENTED,
                        if HIGH_RISK_APPLY_IMPLEMENTED {
                            "high-risk apply support is implemented".to_owned()
                        } else {
                            "high-risk apply is disabled until future support is implemented"
                                .to_owned()
                        },
                        (!HIGH_RISK_APPLY_IMPLEMENTED)
                            .then_some(PolicyRejection::HighRiskApplyNotImplemented),
                    );
                }
            }
        }

        if self.source == ActionSource::RemoteAgent && self.mode.supports_apply() {
            record_remote_apply_rules(self, &mut evaluated_rules, &mut first_rejection);
        }

        match self.mode {
            DaemonMode::Observe | DaemonMode::Suggest => {}
            DaemonMode::ApplyLowRisk => {
                let passed = descriptor.safety_class == SafetyClass::ReversibleLowRisk;
                record_policy_rule(
                    &mut evaluated_rules,
                    &mut first_rejection,
                    "safety_class_allowed",
                    passed,
                    if passed {
                        "safety class ReversibleLowRisk is allowed in daemon mode apply-low-risk"
                            .to_owned()
                    } else {
                        format!(
                            "safety class {:?} is not allowed in daemon mode apply-low-risk",
                            descriptor.safety_class
                        )
                    },
                    (!passed).then_some(PolicyRejection::SafetyClassTooHigh {
                        mode: self.mode,
                        safety_class: descriptor.safety_class.clone(),
                    }),
                );

                let passed = descriptor.effect_scope.is_low_risk_apply_scope();
                record_policy_rule(
                    &mut evaluated_rules,
                    &mut first_rejection,
                    "effect_scope_allowed",
                    passed,
                    if passed {
                        format!(
                            "effect scope {:?} is allowed in daemon mode apply-low-risk",
                            descriptor.effect_scope
                        )
                    } else {
                        format!(
                            "effect scope {:?} is not allowed in daemon mode apply-low-risk",
                            descriptor.effect_scope
                        )
                    },
                    (!passed).then_some(PolicyRejection::EffectScopeNotAllowed {
                        mode: self.mode,
                        effect_scope: descriptor.effect_scope,
                    }),
                );
            }
            DaemonMode::ApplyMediumRisk => {
                let passed = descriptor.safety_class <= SafetyClass::ReversibleMediumRisk;
                record_policy_rule(
                    &mut evaluated_rules,
                    &mut first_rejection,
                    "safety_class_allowed",
                    passed,
                    if passed {
                        format!(
                            "safety class {:?} is allowed in daemon mode apply-medium-risk",
                            descriptor.safety_class
                        )
                    } else {
                        format!(
                            "safety class {:?} is not allowed in daemon mode apply-medium-risk",
                            descriptor.safety_class
                        )
                    },
                    (!passed).then_some(PolicyRejection::SafetyClassTooHigh {
                        mode: self.mode,
                        safety_class: descriptor.safety_class.clone(),
                    }),
                );

                let passed = descriptor.effect_scope.is_explicit_target_scope();
                record_policy_rule(
                    &mut evaluated_rules,
                    &mut first_rejection,
                    "effect_scope_allowed",
                    passed,
                    if passed {
                        format!(
                            "effect scope {:?} is allowed in daemon mode apply-medium-risk",
                            descriptor.effect_scope
                        )
                    } else {
                        format!(
                            "effect scope {:?} is not allowed in daemon mode apply-medium-risk",
                            descriptor.effect_scope
                        )
                    },
                    (!passed).then_some(PolicyRejection::EffectScopeNotAllowed {
                        mode: self.mode,
                        effect_scope: descriptor.effect_scope,
                    }),
                );
            }
            DaemonMode::ApplyHighRisk => {
                record_policy_rule(
                    &mut evaluated_rules,
                    &mut first_rejection,
                    "safety_class_allowed",
                    true,
                    format!(
                        "safety class {:?} is within daemon mode apply-high-risk ceiling",
                        descriptor.safety_class
                    ),
                    None,
                );

                record_policy_rule(
                    &mut evaluated_rules,
                    &mut first_rejection,
                    "effect_scope_allowed",
                    true,
                    format!(
                        "effect scope {:?} is allowed in daemon mode apply-high-risk",
                        descriptor.effect_scope
                    ),
                    None,
                );

                let passed =
                    descriptor.safety_class != SafetyClass::HighRisk || self.allow_high_risk;
                record_policy_rule(
                    &mut evaluated_rules,
                    &mut first_rejection,
                    "high_risk_opt_in",
                    passed,
                    if passed {
                        "high-risk opt-in requirement is satisfied".to_owned()
                    } else {
                        "high-risk apply requires explicit high-risk opt-in".to_owned()
                    },
                    (!passed).then_some(PolicyRejection::HighRiskRequiresExplicitOptIn),
                );
            }
        }

        let passed = !descriptor.touches_system_wide_state || self.allow_system_wide_apply;
        record_policy_rule(
            &mut evaluated_rules,
            &mut first_rejection,
            "system_wide_apply_permission",
            passed,
            if passed {
                "system-wide apply permission is satisfied".to_owned()
            } else {
                "system-wide apply is blocked by daemon policy".to_owned()
            },
            (!passed).then_some(PolicyRejection::SystemWideActionBlocked),
        );

        let passed = !descriptor.persistent_effect || self.allow_persistent_effects;
        record_policy_rule(
            &mut evaluated_rules,
            &mut first_rejection,
            "persistent_effect_permission",
            passed,
            if passed {
                "persistent effect permission is satisfied".to_owned()
            } else {
                "persistent effect is blocked by daemon policy".to_owned()
            },
            (!passed).then_some(PolicyRejection::PersistentEffectBlocked),
        );

        let enabled_passed = self.enabled_action_families.is_empty()
            || self
                .enabled_action_families
                .iter()
                .any(|family| action_kind_matches_family(&descriptor.action_kind, family));
        record_policy_rule(
            &mut evaluated_rules,
            &mut first_rejection,
            "action_family_enabled",
            enabled_passed,
            if enabled_passed {
                "action family is enabled by daemon policy".to_owned()
            } else {
                format!(
                    "action family {} is not enabled by daemon preset",
                    descriptor.action_kind
                )
            },
            (!enabled_passed).then_some(PolicyRejection::ActionFamilyNotEnabled {
                action_kind: descriptor.action_kind.clone(),
            }),
        );

        let denied = self
            .denied_action_families
            .iter()
            .any(|family| action_kind_matches_family(&descriptor.action_kind, family));
        record_policy_rule(
            &mut evaluated_rules,
            &mut first_rejection,
            "action_family_not_denied",
            !denied,
            if denied {
                format!(
                    "action family {} is denied by daemon policy",
                    descriptor.action_kind
                )
            } else {
                "action family is not denied by daemon policy".to_owned()
            },
            denied.then_some(PolicyRejection::ActionFamilyDenied {
                action_kind: descriptor.action_kind.clone(),
            }),
        );

        match descriptor.rollback {
            RollbackRequirement::RequiredBeforeApply => record_policy_rule(
                &mut evaluated_rules,
                &mut first_rejection,
                "rollback_available",
                true,
                "rollback is available before apply".to_owned(),
                None,
            ),
            RollbackRequirement::Unavailable => record_policy_rule(
                &mut evaluated_rules,
                &mut first_rejection,
                "rollback_available",
                false,
                "apply is rejected because rollback is unavailable".to_owned(),
                Some(PolicyRejection::RollbackUnavailable),
            ),
            RollbackRequirement::NotRequiredForDryRun | RollbackRequirement::BestEffortOnly => {
                record_policy_rule(
                    &mut evaluated_rules,
                    &mut first_rejection,
                    "rollback_available",
                    false,
                    format!(
                        "apply requires rollback to be available before apply; got {:?}",
                        descriptor.rollback
                    ),
                    Some(PolicyRejection::RollbackRequired {
                        rollback: descriptor.rollback,
                    }),
                );
            }
        }

        let passed = !descriptor.requires_explicit_target
            || descriptor.effect_scope.is_explicit_target_scope();
        record_policy_rule(
            &mut evaluated_rules,
            &mut first_rejection,
            "explicit_target_scope",
            passed,
            if passed {
                "explicit target requirement is satisfied".to_owned()
            } else {
                "apply requires an explicit target scope".to_owned()
            },
            (!passed).then_some(PolicyRejection::ExplicitTargetRequired),
        );

        if let Some(confidence) = descriptor.confidence {
            let passed = confidence >= self.min_confidence;
            record_policy_rule(
                &mut evaluated_rules,
                &mut first_rejection,
                "confidence_threshold",
                passed,
                if passed {
                    format!(
                        "action confidence {confidence:.3} meets required minimum {:.3}",
                        self.min_confidence
                    )
                } else {
                    format!(
                        "action confidence {confidence:.3} is below required minimum {:.3}",
                        self.min_confidence
                    )
                },
                (!passed).then_some(PolicyRejection::ConfidenceTooLow {
                    confidence,
                    min_confidence: self.min_confidence,
                }),
            );
        } else {
            record_policy_rule(
                &mut evaluated_rules,
                &mut first_rejection,
                "confidence_threshold",
                true,
                "action did not report confidence; threshold check is not required".to_owned(),
                None,
            );
        }

        self.finish_policy_explanation(intent, descriptor, evaluated_rules, first_rejection)
    }

    pub fn check_action(
        &self,
        intent: PolicyIntent,
        descriptor: &ActionDescriptor,
    ) -> Result<(), PolicyRejection> {
        debug_assert!(validate_policy_descriptor_shape(descriptor).is_ok());
        match self.explain_action(intent, descriptor).decision {
            PolicyDecisionKind::Allowed => Ok(()),
            PolicyDecisionKind::Rejected { rejection } => Err(rejection),
        }
    }

    pub fn check_action_with_context(
        &self,
        intent: PolicyIntent,
        descriptor: &ActionDescriptor,
        context: &DaemonPolicyContext,
    ) -> Result<(), PolicyRejection> {
        debug_assert!(validate_policy_descriptor_shape(descriptor).is_ok());
        match self
            .explain_action_with_context(intent, descriptor, context)
            .decision
        {
            PolicyDecisionKind::Allowed => Ok(()),
            PolicyDecisionKind::Rejected { rejection } => Err(rejection),
        }
    }

    pub fn verdict_for_action(
        &self,
        intent: PolicyIntent,
        descriptor: &ActionDescriptor,
    ) -> DaemonPolicyVerdict {
        self.explain_action(intent, descriptor).verdict
    }

    pub fn verdict_for_action_with_context(
        &self,
        intent: PolicyIntent,
        descriptor: &ActionDescriptor,
        context: &DaemonPolicyContext,
    ) -> DaemonPolicyVerdict {
        self.explain_action_with_context(intent, descriptor, context)
            .verdict
    }

    fn record_context_rules(
        &self,
        intent: &PolicyIntent,
        descriptor: &ActionDescriptor,
        context: &DaemonPolicyContext,
        evaluated_rules: &mut Vec<PolicyRuleEvaluation>,
        first_rejection: &mut Option<PolicyRejection>,
    ) {
        let apply_intent = matches!(intent, PolicyIntent::Apply);
        let data_quality_reason = context
            .data_quality_reason_code
            .clone()
            .unwrap_or_else(|| "insufficient_data".to_owned());
        let data_quality_passed = !apply_intent || context.data_quality_ok;
        record_policy_rule(
            evaluated_rules,
            first_rejection,
            "data_quality_gate",
            data_quality_passed,
            if data_quality_passed {
                "data quality gate is satisfied".to_owned()
            } else {
                format!("data quality gate blocks apply: {data_quality_reason}")
            },
            (!data_quality_passed).then_some(PolicyRejection::DataQualityBlocked {
                reason_code: data_quality_reason,
            }),
        );

        let health_reason = context
            .system_health_reason_code
            .clone()
            .unwrap_or_else(|| "system_health_degraded".to_owned());
        let health_passed = !apply_intent || context.system_health_ok;
        record_policy_rule(
            evaluated_rules,
            first_rejection,
            "system_health_gate",
            health_passed,
            if health_passed {
                "system health gate is satisfied".to_owned()
            } else {
                format!("system health gate blocks apply: {health_reason}")
            },
            (!health_passed).then_some(PolicyRejection::SystemHealthBlocked {
                reason_code: health_reason,
            }),
        );

        let workload_passed = !apply_intent || context.workload_stable;
        record_policy_rule(
            evaluated_rules,
            first_rejection,
            "workload_stability_gate",
            workload_passed,
            if workload_passed {
                "workload stability gate is satisfied".to_owned()
            } else {
                "workload identity is unstable".to_owned()
            },
            (!workload_passed).then_some(PolicyRejection::WorkloadUnstable),
        );

        let cooldown_passed = !apply_intent || !context.cooldown_active;
        record_policy_rule(
            evaluated_rules,
            first_rejection,
            "cooldown_gate",
            cooldown_passed,
            if cooldown_passed {
                "cooldown gate is satisfied".to_owned()
            } else {
                "cooldown is active".to_owned()
            },
            (!cooldown_passed).then_some(PolicyRejection::CooldownActive),
        );

        let rollback_passed = !apply_intent || !context.rollback_pending;
        record_policy_rule(
            evaluated_rules,
            first_rejection,
            "rollback_pending_gate",
            rollback_passed,
            if rollback_passed {
                "rollback pending gate is satisfied".to_owned()
            } else {
                "a previous rollback is pending".to_owned()
            },
            (!rollback_passed).then_some(PolicyRejection::RollbackPending),
        );

        let capability_checked = matches!(
            intent,
            PolicyIntent::Suggest | PolicyIntent::DryRun | PolicyIntent::Apply
        );
        let missing_capability = context.capabilities.as_ref().and_then(|capabilities| {
            unavailable_capability_for_action(&descriptor.action_kind, capabilities)
        });
        let capability_passed = !capability_checked || missing_capability.is_none();
        record_policy_rule(
            evaluated_rules,
            first_rejection,
            "capability_gate",
            capability_passed,
            if let Some(feature) = missing_capability {
                format!(
                    "action {} requires unavailable daemon capability: {feature}",
                    descriptor.action_kind
                )
            } else if context.capabilities.is_some() {
                "daemon capability gate is satisfied".to_owned()
            } else {
                "daemon capabilities were not provided for this policy check".to_owned()
            },
            (!capability_passed).then(|| PolicyRejection::CapabilityUnavailable {
                action_kind: descriptor.action_kind.clone(),
                feature: missing_capability.expect("missing capability is required on rejection"),
            }),
        );
    }

    fn finish_policy_explanation(
        &self,
        intent: PolicyIntent,
        descriptor: &ActionDescriptor,
        evaluated_rules: Vec<PolicyRuleEvaluation>,
        rejection: Option<PolicyRejection>,
    ) -> PolicyExplanation {
        let (verdict, decision, final_reason) = match rejection {
            Some(rejection) => {
                let verdict = rejection.verdict();
                let final_reason = rejection.to_string();
                (
                    verdict,
                    PolicyDecisionKind::Rejected { rejection },
                    final_reason,
                )
            }
            None => (
                DaemonPolicyVerdict::Allow,
                PolicyDecisionKind::Allowed,
                "action is allowed by daemon policy".to_owned(),
            ),
        };

        PolicyExplanation {
            verdict,
            decision,
            intent,
            action_id: descriptor.action_id.clone(),
            action_kind: descriptor.action_kind.clone(),
            mode: self.mode,
            source: self.source,
            evaluated_rules,
            final_reason,
        }
    }
}

fn validate_policy_descriptor_shape(
    descriptor: &ActionDescriptor,
) -> Result<(), DaemonPolicyError> {
    if descriptor.action_kind.trim().is_empty() {
        return Err(DaemonPolicyError::InvalidInput {
            message: "action_kind must not be empty".to_owned(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(safety_class: SafetyClass) -> ActionDescriptor {
        descriptor_with(
            safety_class,
            ActionEffectScope::LocalProcessTree,
            RollbackRequirement::RequiredBeforeApply,
        )
    }

    fn descriptor_with(
        safety_class: SafetyClass,
        effect_scope: ActionEffectScope,
        rollback: RollbackRequirement,
    ) -> ActionDescriptor {
        ActionDescriptor {
            action_id: ActionId("test-action".to_owned()),
            action_kind: "test".to_owned(),
            safety_class,
            effect_scope,
            rollback,
            persistent_effect: false,
            touches_system_wide_state: false,
            requires_explicit_target: true,
            confidence: Some(0.90),
        }
    }

    fn all_safety_classes() -> [SafetyClass; 4] {
        [
            SafetyClass::ObserveOnly,
            SafetyClass::ReversibleLowRisk,
            SafetyClass::ReversibleMediumRisk,
            SafetyClass::HighRisk,
        ]
    }

    fn all_capabilities_available() -> DaemonCapabilities {
        DaemonCapabilities {
            kernel_release: Some("6.9.1-test".to_owned()),
            btf_available: true,
            sched_tracepoints_available: true,
            perf_permissions_likely: true,
            perf_event_paranoid: Some(1),
            cgroup_v2_available: true,
            sched_ext_available: true,
            uclamp_available: true,
            ionice_available: true,
            irq_affinity_available: true,
            gpu_sysfs_available: true,
        }
    }

    #[test]
    fn daemon_mode_parses_and_formats_kebab_case() {
        assert_eq!(
            "observe".parse::<DaemonMode>().unwrap(),
            DaemonMode::Observe
        );
        assert_eq!(
            "suggest".parse::<DaemonMode>().unwrap(),
            DaemonMode::Suggest
        );
        assert_eq!(
            "apply-low-risk".parse::<DaemonMode>().unwrap(),
            DaemonMode::ApplyLowRisk
        );
        assert_eq!(
            "apply-medium-risk".parse::<DaemonMode>().unwrap(),
            DaemonMode::ApplyMediumRisk
        );
        assert_eq!(
            "apply-high-risk".parse::<DaemonMode>().unwrap(),
            DaemonMode::ApplyHighRisk
        );
        assert_eq!(DaemonMode::ApplyHighRisk.to_string(), "apply-high-risk");
        assert!(matches!(
            "bad-mode".parse::<DaemonMode>(),
            Err(PolicyRejection::UnsupportedMode { .. })
        ));
    }

    #[test]
    fn daemon_mode_serializes_as_kebab_case() {
        let json = serde_json::to_string(&DaemonMode::ApplyMediumRisk).unwrap();

        assert_eq!(json, "\"apply-medium-risk\"");
    }

    #[test]
    fn new_policy_fields_are_populated_by_existing_constructors() {
        let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);

        assert_eq!(policy.max_safety_class, SafetyClass::ReversibleLowRisk);
        assert!(
            policy
                .allowed_effect_scopes
                .contains(&ActionEffectScope::LocalProcess)
        );
        assert!(
            policy
                .allowed_effect_scopes
                .contains(&ActionEffectScope::LocalProcessTree)
        );
        assert!(policy.rollback_required_before_apply);
        assert!(!policy.allow_system_wide_suggestions);
        assert!(!policy.allow_system_wide_apply);
        assert!(!policy.allow_high_risk);
        assert!(!policy.allow_persistent_effects);
        assert_eq!(policy.confidence.min_suggest_confidence, 0.50);
        assert_eq!(
            policy.confidence.min_apply_low_risk_confidence,
            policy.min_confidence
        );
        assert_eq!(policy.confidence.min_apply_medium_risk_confidence, 0.85);
        assert_eq!(policy.confidence.min_high_risk_suggestion_confidence, 0.90);
        assert!(!policy.remote_apply.allow_remote_apply);
    }

    #[test]
    fn observe_rejects_apply_for_all_safety_classes() {
        let policy = DaemonPolicy::observe(ActionSource::Test);

        for safety_class in all_safety_classes() {
            let desc = descriptor(safety_class);
            assert!(matches!(
                policy.check_action(PolicyIntent::Apply, &desc),
                Err(PolicyRejection::IntentNotAllowed {
                    mode: DaemonMode::Observe,
                    intent: PolicyIntent::Apply
                })
            ));
        }
    }

    #[test]
    fn observe_allows_dry_run_for_all_safety_classes() {
        let policy = DaemonPolicy::observe(ActionSource::Test);

        for safety_class in all_safety_classes() {
            let desc = descriptor_with(
                safety_class,
                ActionEffectScope::SystemWide,
                RollbackRequirement::Unavailable,
            );
            assert!(policy.check_action(PolicyIntent::DryRun, &desc).is_ok());
        }
    }

    #[test]
    fn suggest_rejects_apply_for_all_safety_classes() {
        let policy = DaemonPolicy::suggest(ActionSource::Test);

        for safety_class in all_safety_classes() {
            let desc = descriptor(safety_class);
            assert!(matches!(
                policy.check_action(PolicyIntent::Apply, &desc),
                Err(PolicyRejection::IntentNotAllowed {
                    mode: DaemonMode::Suggest,
                    intent: PolicyIntent::Apply
                })
            ));
        }
    }

    #[test]
    fn suggest_rejects_system_wide_suggestion_without_permission() {
        let policy = DaemonPolicy::suggest(ActionSource::Test);
        let mut desc = descriptor_with(
            SafetyClass::HighRisk,
            ActionEffectScope::SystemWide,
            RollbackRequirement::Unavailable,
        );
        desc.touches_system_wide_state = true;

        assert!(matches!(
            policy.check_action(PolicyIntent::Suggest, &desc),
            Err(PolicyRejection::SystemWideActionBlocked)
        ));
    }

    #[test]
    fn suggest_allows_system_wide_suggestion_with_suggestion_permission() {
        let mut config = crate::daemon::config::DaemonConfig {
            mode: DaemonMode::Suggest,
            source: ActionSource::Test,
            ..crate::daemon::config::DaemonConfig::default()
        };
        config.safety.allow_system_wide_suggestions = true;
        let policy = build_daemon_policy(DaemonPolicyBuildInput {
            config: &config,
            remote_context: None,
        });
        let mut desc = descriptor_with(
            SafetyClass::HighRisk,
            ActionEffectScope::SystemWide,
            RollbackRequirement::Unavailable,
        );
        desc.touches_system_wide_state = true;

        assert!(policy.check_action(PolicyIntent::Suggest, &desc).is_ok());
        assert!(policy.allow_system_wide_suggestions);
        assert!(!policy.allow_system_wide_apply);
    }

    #[test]
    fn apply_low_risk_allows_reversible_low_risk_local_process_tree_with_required_rollback() {
        let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
        let desc = descriptor_with(
            SafetyClass::ReversibleLowRisk,
            ActionEffectScope::LocalProcessTree,
            RollbackRequirement::RequiredBeforeApply,
        );

        assert!(policy.check_action(PolicyIntent::Apply, &desc).is_ok());
    }

    #[test]
    fn apply_low_risk_rejects_medium_risk() {
        let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
        let desc = descriptor(SafetyClass::ReversibleMediumRisk);

        assert!(matches!(
            policy.check_action(PolicyIntent::Apply, &desc),
            Err(PolicyRejection::SafetyClassTooHigh {
                mode: DaemonMode::ApplyLowRisk,
                safety_class: SafetyClass::ReversibleMediumRisk
            })
        ));
    }

    #[test]
    fn apply_low_risk_rejects_high_risk() {
        let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
        let desc = descriptor(SafetyClass::HighRisk);

        assert!(matches!(
            policy.check_action(PolicyIntent::Apply, &desc),
            Err(PolicyRejection::SafetyClassTooHigh {
                mode: DaemonMode::ApplyLowRisk,
                safety_class: SafetyClass::HighRisk
            })
        ));
    }

    #[test]
    fn apply_low_risk_rejects_system_wide_even_when_low_risk() {
        let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
        let mut desc = descriptor_with(
            SafetyClass::ReversibleLowRisk,
            ActionEffectScope::LocalProcessTree,
            RollbackRequirement::RequiredBeforeApply,
        );
        desc.touches_system_wide_state = true;

        assert!(matches!(
            policy.check_action(PolicyIntent::Apply, &desc),
            Err(PolicyRejection::SystemWideActionBlocked)
        ));
    }

    #[test]
    fn apply_low_risk_rejects_missing_rollback() {
        let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
        let desc = descriptor_with(
            SafetyClass::ReversibleLowRisk,
            ActionEffectScope::LocalProcessTree,
            RollbackRequirement::BestEffortOnly,
        );

        assert!(matches!(
            policy.check_action(PolicyIntent::Apply, &desc),
            Err(PolicyRejection::RollbackRequired {
                rollback: RollbackRequirement::BestEffortOnly
            })
        ));
    }

    #[test]
    fn apply_low_risk_rejects_low_confidence() {
        let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
        let mut desc = descriptor(SafetyClass::ReversibleLowRisk);
        desc.confidence = Some(0.10);

        assert!(matches!(
            policy.check_action(PolicyIntent::Apply, &desc),
            Err(PolicyRejection::ConfidenceTooLow { .. })
        ));
    }

    #[test]
    fn policy_verdicts_distinguish_delay_observe_only_manual_and_reject() {
        let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
        let desc = descriptor(SafetyClass::ReversibleLowRisk);

        assert_eq!(
            policy.verdict_for_action_with_context(
                PolicyIntent::Apply,
                &desc,
                &DaemonPolicyContext {
                    cooldown_active: true,
                    ..DaemonPolicyContext::default()
                },
            ),
            DaemonPolicyVerdict::Delay
        );
        assert_eq!(
            policy.verdict_for_action_with_context(
                PolicyIntent::Apply,
                &desc,
                &DaemonPolicyContext {
                    system_health_ok: false,
                    system_health_reason_code: Some("thermal_degraded".to_owned()),
                    ..DaemonPolicyContext::default()
                },
            ),
            DaemonPolicyVerdict::RequireObserveOnly
        );

        let mut high_risk = descriptor(SafetyClass::HighRisk);
        high_risk.confidence = Some(1.0);
        assert_eq!(
            policy.verdict_for_action(PolicyIntent::Apply, &high_risk),
            DaemonPolicyVerdict::RequireManualConfirmation
        );

        let no_rollback = descriptor_with(
            SafetyClass::ReversibleLowRisk,
            ActionEffectScope::LocalProcessTree,
            RollbackRequirement::Unavailable,
        );
        assert_eq!(
            policy.verdict_for_action(PolicyIntent::Apply, &no_rollback),
            DaemonPolicyVerdict::Reject
        );
    }

    #[test]
    fn policy_explanation_exposes_verdict_for_machine_clients() {
        let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
        let desc = descriptor(SafetyClass::ReversibleLowRisk);

        let allowed = policy.explain_action(PolicyIntent::Apply, &desc);
        let delayed = policy.explain_action_with_context(
            PolicyIntent::Apply,
            &desc,
            &DaemonPolicyContext {
                cooldown_active: true,
                ..DaemonPolicyContext::default()
            },
        );

        assert_eq!(allowed.verdict, DaemonPolicyVerdict::Allow);
        assert!(allowed.verdict.is_allowed());
        assert_eq!(delayed.verdict, DaemonPolicyVerdict::Delay);
        assert!(!delayed.verdict.is_allowed());
        assert_eq!(DaemonPolicyVerdict::Delay.as_str(), "delay");
    }

    #[test]
    fn daemon_policy_context_blocks_apply_on_bad_data_quality() {
        let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
        let desc = descriptor(SafetyClass::ReversibleLowRisk);
        let context = DaemonPolicyContext {
            data_quality_ok: false,
            data_quality_reason_code: Some("insufficient_samples".to_owned()),
            ..DaemonPolicyContext::default()
        };

        let explanation = policy.explain_action_with_context(PolicyIntent::Apply, &desc, &context);

        assert!(matches!(
            explanation.decision,
            PolicyDecisionKind::Rejected {
                rejection: PolicyRejection::DataQualityBlocked { .. }
            }
        ));
        assert_eq!(
            policy
                .check_action_with_context(PolicyIntent::Apply, &desc, &context)
                .unwrap_err()
                .reason_code(),
            "data_quality_blocked"
        );
        assert!(
            explanation
                .evaluated_rules
                .iter()
                .any(|rule| rule.rule == "data_quality_gate" && !rule.passed)
        );
    }

    #[test]
    fn daemon_policy_context_blocks_apply_when_health_or_state_is_unsafe() {
        let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
        let desc = descriptor(SafetyClass::ReversibleLowRisk);

        for (context, expected) in [
            (
                DaemonPolicyContext {
                    system_health_ok: false,
                    system_health_reason_code: Some("thermal_degraded".to_owned()),
                    ..DaemonPolicyContext::default()
                },
                "system_health_blocked",
            ),
            (
                DaemonPolicyContext {
                    workload_stable: false,
                    ..DaemonPolicyContext::default()
                },
                "workload_unstable",
            ),
            (
                DaemonPolicyContext {
                    cooldown_active: true,
                    ..DaemonPolicyContext::default()
                },
                "cooldown_active",
            ),
            (
                DaemonPolicyContext {
                    rollback_pending: true,
                    ..DaemonPolicyContext::default()
                },
                "rollback_pending",
            ),
        ] {
            let rejection = policy
                .check_action_with_context(PolicyIntent::Apply, &desc, &context)
                .unwrap_err();

            assert_eq!(rejection.reason_code(), expected);
        }
    }

    #[test]
    fn daemon_policy_context_can_be_derived_from_health_snapshot() {
        let context = DaemonPolicyContext::default().with_system_health(
            &crate::daemon::health::SystemHealthSnapshot {
                ok_for_apply: false,
                reason_code: Some("low_disk".to_owned()),
                ..crate::daemon::health::SystemHealthSnapshot::default()
            },
        );

        assert!(!context.system_health_ok);
        assert_eq!(
            context.system_health_reason_code.as_deref(),
            Some("low_disk")
        );
    }

    #[test]
    fn daemon_policy_context_does_not_block_observe_intent() {
        let policy = DaemonPolicy::observe(ActionSource::Test);
        let desc = descriptor_with(
            SafetyClass::HighRisk,
            ActionEffectScope::SystemWide,
            RollbackRequirement::Unavailable,
        );
        let context = DaemonPolicyContext {
            data_quality_ok: false,
            system_health_ok: false,
            workload_stable: false,
            cooldown_active: true,
            rollback_pending: true,
            ..DaemonPolicyContext::default()
        };

        let explanation =
            policy.explain_action_with_context(PolicyIntent::Observe, &desc, &context);

        assert!(matches!(explanation.decision, PolicyDecisionKind::Allowed));
        assert!(
            explanation
                .evaluated_rules
                .iter()
                .filter(|rule| rule.rule.ends_with("_gate"))
                .all(|rule| rule.passed)
        );
    }

    #[test]
    fn daemon_policy_context_blocks_unsupported_action_capabilities() {
        let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
        let mut desc = descriptor(SafetyClass::ReversibleLowRisk);
        desc.action_kind = "uclamp:min".to_owned();
        let mut capabilities = all_capabilities_available();
        capabilities.uclamp_available = false;
        let context = DaemonPolicyContext {
            capabilities: Some(capabilities),
            ..DaemonPolicyContext::default()
        };

        let explanation = policy.explain_action_with_context(PolicyIntent::Apply, &desc, &context);

        assert!(matches!(
            explanation.decision,
            PolicyDecisionKind::Rejected {
                rejection: PolicyRejection::CapabilityUnavailable {
                    feature: "uclamp",
                    ..
                }
            }
        ));
        assert!(
            explanation
                .evaluated_rules
                .iter()
                .any(|rule| rule.rule == "capability_gate" && !rule.passed)
        );
    }

    #[test]
    fn daemon_policy_context_uses_capabilities_for_dry_run_and_suggest_but_not_observe() {
        let policy = DaemonPolicy::suggest(ActionSource::Test);
        let mut desc = descriptor_with(
            SafetyClass::ReversibleLowRisk,
            ActionEffectScope::LocalProcessTree,
            RollbackRequirement::RequiredBeforeApply,
        );
        desc.action_kind = "ionice".to_owned();
        let mut capabilities = all_capabilities_available();
        capabilities.ionice_available = false;
        let context = DaemonPolicyContext {
            capabilities: Some(capabilities),
            ..DaemonPolicyContext::default()
        };

        let dry_run_rejection = policy
            .check_action_with_context(PolicyIntent::DryRun, &desc, &context)
            .unwrap_err();
        let suggest_rejection = policy
            .check_action_with_context(PolicyIntent::Suggest, &desc, &context)
            .unwrap_err();
        let observe = policy.check_action_with_context(PolicyIntent::Observe, &desc, &context);

        assert_eq!(dry_run_rejection.reason_code(), "capability_unavailable");
        assert_eq!(suggest_rejection.reason_code(), "capability_unavailable");
        assert!(observe.is_ok());
    }

    #[test]
    fn apply_medium_risk_allows_medium_risk_only_when_explicit() {
        let medium_desc = descriptor(SafetyClass::ReversibleMediumRisk);
        let low_policy = DaemonPolicy::apply_low_risk(ActionSource::Test);

        assert!(matches!(
            low_policy.check_action(PolicyIntent::Apply, &medium_desc),
            Err(PolicyRejection::SafetyClassTooHigh {
                mode: DaemonMode::ApplyLowRisk,
                ..
            })
        ));

        let medium_policy = DaemonPolicy::apply_medium_risk(ActionSource::Test);
        assert!(
            medium_policy
                .check_action(PolicyIntent::Apply, &medium_desc)
                .is_ok()
        );
    }

    #[test]
    fn apply_high_risk_is_disabled_even_with_explicit_high_risk_unlock() {
        let mut policy = DaemonPolicy::apply_high_risk_explicit(ActionSource::Test);
        policy.allow_high_risk = false;
        let high = descriptor(SafetyClass::HighRisk);

        assert!(matches!(
            policy.check_action(PolicyIntent::Apply, &high),
            Err(PolicyRejection::HighRiskApplyNotImplemented)
        ));

        let explicit = DaemonPolicy::apply_high_risk_explicit(ActionSource::Test);
        assert!(matches!(
            explicit.check_action(PolicyIntent::Apply, &high),
            Err(PolicyRejection::HighRiskApplyNotImplemented)
        ));
    }

    #[test]
    fn apply_rejects_persistent_effect_without_explicit_permission() {
        let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
        let mut desc = descriptor(SafetyClass::ReversibleLowRisk);
        desc.persistent_effect = true;

        assert!(matches!(
            policy.check_action(PolicyIntent::Apply, &desc),
            Err(PolicyRejection::PersistentEffectBlocked)
        ));
    }

    #[test]
    fn apply_rejects_unavailable_rollback() {
        let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
        let desc = descriptor_with(
            SafetyClass::ReversibleLowRisk,
            ActionEffectScope::LocalProcessTree,
            RollbackRequirement::Unavailable,
        );

        assert!(matches!(
            policy.check_action(PolicyIntent::Apply, &desc),
            Err(PolicyRejection::RollbackUnavailable)
        ));
    }

    #[test]
    fn explain_action_allowed_includes_identity_final_reason_and_rules() {
        let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
        let desc = descriptor_with(
            SafetyClass::ReversibleLowRisk,
            ActionEffectScope::LocalProcessTree,
            RollbackRequirement::RequiredBeforeApply,
        );

        let explanation = policy.explain_action(PolicyIntent::Apply, &desc);

        assert!(matches!(explanation.decision, PolicyDecisionKind::Allowed));
        assert_eq!(explanation.intent, PolicyIntent::Apply);
        assert_eq!(explanation.action_id, desc.action_id);
        assert_eq!(explanation.action_kind, "test");
        assert_eq!(explanation.mode, DaemonMode::ApplyLowRisk);
        assert_eq!(explanation.source, ActionSource::Test);
        assert_eq!(
            explanation.final_reason,
            "action is allowed by daemon policy"
        );
        assert!(
            explanation
                .evaluated_rules
                .iter()
                .any(|rule| { rule.rule == "rollback_available" && rule.passed })
        );
    }

    #[test]
    fn explain_action_rejection_includes_identity_final_reason_and_failing_rule() {
        let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
        let desc = descriptor(SafetyClass::ReversibleMediumRisk);

        let explanation = policy.explain_action(PolicyIntent::Apply, &desc);

        assert!(matches!(
            explanation.decision,
            PolicyDecisionKind::Rejected {
                rejection: PolicyRejection::SafetyClassTooHigh {
                    mode: DaemonMode::ApplyLowRisk,
                    safety_class: SafetyClass::ReversibleMediumRisk
                }
            }
        ));
        assert_eq!(explanation.intent, PolicyIntent::Apply);
        assert_eq!(explanation.action_id, desc.action_id);
        assert_eq!(explanation.action_kind, "test");
        assert_eq!(explanation.mode, DaemonMode::ApplyLowRisk);
        assert_eq!(explanation.source, ActionSource::Test);
        assert!(
            explanation
                .final_reason
                .contains("safety class ReversibleMediumRisk")
        );

        let failing_rule = explanation
            .evaluated_rules
            .iter()
            .find(|rule| !rule.passed)
            .expect("expected a failing policy rule");
        assert_eq!(failing_rule.rule, "safety_class_allowed");
        assert!(failing_rule.reason.contains("ReversibleMediumRisk"));
    }

    #[test]
    fn build_daemon_policy_is_deterministic_for_same_config() {
        let mut config = crate::daemon::config::DaemonConfig {
            mode: DaemonMode::ApplyMediumRisk,
            source: ActionSource::Test,
            ..crate::daemon::config::DaemonConfig::default()
        };
        config.safety.allow_system_wide_suggestions = true;
        config.safety.allow_system_wide_apply = true;
        config.safety.allow_persistent_effects = true;
        config
            .safety
            .denied_action_families
            .insert("gpu-power".to_owned());

        let first = build_daemon_policy(DaemonPolicyBuildInput {
            config: &config,
            remote_context: None,
        });
        let second = build_daemon_policy(DaemonPolicyBuildInput {
            config: &config,
            remote_context: None,
        });

        assert_eq!(first, second);
        assert_eq!(first.mode, DaemonMode::ApplyMediumRisk);
        assert_eq!(first.max_safety_class, SafetyClass::ReversibleMediumRisk);
        assert!(first.allow_system_wide_suggestions);
        assert!(!first.allow_system_wide_apply);
        assert!(first.allow_persistent_effects);
        assert!(first.denied_action_families.contains("gpu-power"));
    }

    #[test]
    fn preset_enabled_action_families_gate_apply_actions() {
        let config = crate::daemon::config::DaemonConfig::from_preset(
            crate::daemon::config::DaemonPreset::GamingLowRisk,
            ActionSource::Test,
        );
        let policy = build_daemon_policy(DaemonPolicyBuildInput {
            config: &config,
            remote_context: None,
        });

        let mut cpu = descriptor(SafetyClass::ReversibleLowRisk);
        cpu.action_kind = "cpu_affinity_profile".to_owned();
        assert!(policy.check_action(PolicyIntent::Apply, &cpu).is_ok());

        let mut nice = descriptor(SafetyClass::ReversibleLowRisk);
        nice.action_kind = "nice".to_owned();
        let rejection = policy.check_action(PolicyIntent::Apply, &nice).unwrap_err();

        assert_eq!(rejection.reason_code(), "action_family_not_enabled");
    }

    #[test]
    fn denied_action_families_gate_apply_actions() {
        let mut config = crate::daemon::config::DaemonConfig {
            mode: DaemonMode::ApplyLowRisk,
            source: ActionSource::Test,
            ..crate::daemon::config::DaemonConfig::default()
        };
        config
            .safety
            .denied_action_families
            .insert("cpu_affinity_profile".to_owned());
        let policy = build_daemon_policy(DaemonPolicyBuildInput {
            config: &config,
            remote_context: None,
        });
        let mut desc = descriptor(SafetyClass::ReversibleLowRisk);
        desc.action_kind = "cpu_affinity_profile".to_owned();

        let rejection = policy.check_action(PolicyIntent::Apply, &desc).unwrap_err();

        assert_eq!(rejection.reason_code(), "action_family_denied");
    }

    #[test]
    fn build_daemon_policy_remote_context_is_deterministic_and_respects_limits() {
        let mut config = crate::daemon::config::DaemonConfig {
            mode: DaemonMode::ApplyLowRisk,
            source: ActionSource::RemoteAgent,
            ..crate::daemon::config::DaemonConfig::default()
        };
        config.remote.allow_remote_apply = true;
        config.safety.allow_system_wide_suggestions = true;
        config.safety.allow_system_wide_apply = true;

        let remote_context = RemotePolicyContext {
            bind_is_loopback: true,
            auth_configured: true,
            request_authorized: true,
            limits: AgentAutotuneLimits {
                max_targets: 3,
                ..AgentAutotuneLimits::default()
            },
        };

        let first = build_daemon_policy(DaemonPolicyBuildInput {
            config: &config,
            remote_context: Some(remote_context.clone()),
        });
        let second = build_daemon_policy(DaemonPolicyBuildInput {
            config: &config,
            remote_context: Some(remote_context),
        });

        assert_eq!(first, second);
        assert!(first.remote_apply.allow_remote_apply);
        assert!(first.remote_apply.require_loopback_bind);
        assert!(first.remote_apply.require_auth);
        assert_eq!(first.remote_apply.max_remote_targets, 3);
        assert!(!first.allow_system_wide_suggestions);
        assert!(!first.allow_system_wide_apply);
    }

    #[test]
    fn remote_limits_cap_system_wide_suggestion_and_apply_separately() {
        let mut config = crate::daemon::config::DaemonConfig {
            mode: DaemonMode::ApplyHighRisk,
            source: ActionSource::RemoteAgent,
            ..crate::daemon::config::DaemonConfig::default()
        };
        config.remote.allow_remote_apply = true;
        config.safety.allow_high_risk = true;
        config.safety.allow_system_wide_suggestions = true;
        config.safety.allow_system_wide_apply = true;

        let suggestion_only_context = RemotePolicyContext {
            bind_is_loopback: true,
            auth_configured: true,
            request_authorized: true,
            limits: AgentAutotuneLimits {
                allow_system_wide_suggestions: true,
                allow_system_wide_apply: false,
                allow_high_risk: true,
                max_mode: DaemonMode::ApplyHighRisk,
                max_safety_class: SafetyClass::HighRisk,
                ..AgentAutotuneLimits::default()
            },
        };

        let policy = build_daemon_policy(DaemonPolicyBuildInput {
            config: &config,
            remote_context: Some(suggestion_only_context),
        });

        assert!(policy.allow_system_wide_suggestions);
        assert!(!policy.allow_system_wide_apply);
    }

    #[test]
    fn remote_apply_explanation_rejects_unconfigured_auth() {
        let mut config = crate::daemon::config::DaemonConfig {
            mode: DaemonMode::ApplyLowRisk,
            source: ActionSource::RemoteAgent,
            ..crate::daemon::config::DaemonConfig::default()
        };
        config.remote.allow_remote_apply = true;
        config.target.tree_pids.push(1234);

        let policy = build_daemon_policy(DaemonPolicyBuildInput {
            config: &config,
            remote_context: Some(RemotePolicyContext {
                bind_is_loopback: true,
                auth_configured: false,
                request_authorized: true,
                limits: AgentAutotuneLimits::default(),
            }),
        });
        let desc = descriptor(SafetyClass::ReversibleLowRisk);

        let explanation = policy.explain_action(PolicyIntent::Apply, &desc);

        assert!(matches!(
            explanation.decision,
            PolicyDecisionKind::Rejected {
                rejection: PolicyRejection::RemoteApplyRequiresConfiguredAuth
            }
        ));
        assert!(
            explanation
                .evaluated_rules
                .iter()
                .any(|rule| rule.rule == "remote_auth_configured" && !rule.passed)
        );
        assert!(explanation.final_reason.contains("configured bearer token"));
    }

    #[test]
    fn remote_apply_explanation_rejects_non_loopback_bind() {
        let mut config = crate::daemon::config::DaemonConfig {
            mode: DaemonMode::ApplyLowRisk,
            source: ActionSource::RemoteAgent,
            ..crate::daemon::config::DaemonConfig::default()
        };
        config.remote.allow_remote_apply = true;
        config.target.tree_pids.push(1234);

        let policy = build_daemon_policy(DaemonPolicyBuildInput {
            config: &config,
            remote_context: Some(RemotePolicyContext {
                bind_is_loopback: false,
                auth_configured: true,
                request_authorized: true,
                limits: AgentAutotuneLimits::default(),
            }),
        });
        let desc = descriptor(SafetyClass::ReversibleLowRisk);

        let explanation = policy.explain_action(PolicyIntent::Apply, &desc);

        assert!(matches!(
            explanation.decision,
            PolicyDecisionKind::Rejected {
                rejection: PolicyRejection::RemoteApplyRequiresLoopbackBind
            }
        ));
        assert!(
            explanation
                .evaluated_rules
                .iter()
                .any(|rule| rule.rule == "remote_loopback_bind" && !rule.passed)
        );
        assert!(explanation.final_reason.contains("loopback"));
    }

    #[test]
    fn remote_apply_high_risk_reports_disabled_before_remote_auth() {
        let mut config = crate::daemon::config::DaemonConfig {
            mode: DaemonMode::ApplyHighRisk,
            source: ActionSource::RemoteAgent,
            ..crate::daemon::config::DaemonConfig::default()
        };
        config.remote.allow_remote_apply = true;
        config.safety.allow_high_risk = true;
        config.target.tree_pids.push(1234);

        let policy = build_daemon_policy(DaemonPolicyBuildInput {
            config: &config,
            remote_context: Some(RemotePolicyContext {
                bind_is_loopback: true,
                auth_configured: false,
                request_authorized: true,
                limits: AgentAutotuneLimits::default(),
            }),
        });
        let desc = descriptor(SafetyClass::HighRisk);

        let explanation = policy.explain_action(PolicyIntent::Apply, &desc);

        assert!(matches!(
            explanation.decision,
            PolicyDecisionKind::Rejected {
                rejection: PolicyRejection::HighRiskApplyNotImplemented
            }
        ));
        let first_failed_rule = explanation
            .evaluated_rules
            .iter()
            .find(|rule| !rule.passed)
            .expect("expected a failing remote policy rule");
        assert_eq!(first_failed_rule.rule, "high_risk_apply_support");
        assert!(
            explanation
                .final_reason
                .contains("high-risk apply is not implemented")
        );
    }

    #[test]
    fn remote_apply_explanation_rejects_mode_over_limits() {
        let mut config = crate::daemon::config::DaemonConfig {
            mode: DaemonMode::ApplyHighRisk,
            source: ActionSource::RemoteAgent,
            ..crate::daemon::config::DaemonConfig::default()
        };
        config.remote.allow_remote_apply = true;
        config.safety.allow_high_risk = true;
        config.target.tree_pids.push(1234);

        let policy = build_daemon_policy(DaemonPolicyBuildInput {
            config: &config,
            remote_context: Some(RemotePolicyContext {
                bind_is_loopback: true,
                auth_configured: true,
                request_authorized: true,
                limits: AgentAutotuneLimits::default(),
            }),
        });
        let desc = descriptor(SafetyClass::HighRisk);

        let explanation = policy.explain_action(PolicyIntent::Apply, &desc);

        assert!(matches!(
            explanation.decision,
            PolicyDecisionKind::Rejected {
                rejection: PolicyRejection::HighRiskApplyNotImplemented
            }
        ));
        assert!(
            explanation
                .evaluated_rules
                .iter()
                .any(|rule| rule.rule == "high_risk_apply_support" && !rule.passed)
        );
        assert!(
            explanation
                .final_reason
                .contains("high-risk apply is not implemented")
        );
    }

    #[test]
    fn remote_apply_explanation_rejects_target_count_over_limits() {
        let mut config = crate::daemon::config::DaemonConfig {
            mode: DaemonMode::ApplyLowRisk,
            source: ActionSource::RemoteAgent,
            ..crate::daemon::config::DaemonConfig::default()
        };
        config.remote.allow_remote_apply = true;
        config.target.tree_pids.push(1234);
        config.target.watch_process = Some("game".to_owned());

        let policy = build_daemon_policy(DaemonPolicyBuildInput {
            config: &config,
            remote_context: Some(RemotePolicyContext {
                bind_is_loopback: true,
                auth_configured: true,
                request_authorized: true,
                limits: AgentAutotuneLimits::default(),
            }),
        });
        let desc = descriptor(SafetyClass::ReversibleLowRisk);

        let explanation = policy.explain_action(PolicyIntent::Apply, &desc);

        assert!(matches!(
            explanation.decision,
            PolicyDecisionKind::Rejected {
                rejection: PolicyRejection::RemoteTargetCountTooHigh {
                    target_count: 2,
                    max_targets: 1
                }
            }
        ));
        assert!(
            explanation
                .evaluated_rules
                .iter()
                .any(|rule| rule.rule == "remote_target_count" && !rule.passed)
        );
        assert!(explanation.final_reason.contains("max_targets"));
    }
}
