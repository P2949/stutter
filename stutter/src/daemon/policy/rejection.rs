//! Daemon policy rejection reasons and machine-readable rejection metadata.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::{
    ActionEffectScope, DaemonMode, DaemonPolicyVerdict, PolicyIntent, RollbackRequirement,
};
use crate::actions::SafetyClass;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PolicyRejection {
    UnsupportedMode {
        mode: String,
    },
    InvalidActionDescriptor {
        reason: String,
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
            Self::InvalidActionDescriptor { reason } => {
                write!(f, "invalid action descriptor: {reason}")
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
            | Self::InvalidActionDescriptor { .. }
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
            Self::InvalidActionDescriptor { .. } => "invalid_action_descriptor",
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
