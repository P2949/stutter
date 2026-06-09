//! Daemon-policy mapping helpers for planner evaluation; this module owns policy translation, not candidate scoring.

use crate::{
    autotune::planning::{denial::CandidateDenyReason, planner::PlannerInput},
    daemon::{DaemonPolicy, policy::DaemonMode},
    daemon_policy::{DaemonPolicyContext, PolicyIntent},
};

pub(crate) fn policy_intent_for_mode(mode: DaemonMode) -> PolicyIntent {
    if mode.supports_apply() {
        PolicyIntent::Apply
    } else {
        PolicyIntent::Suggest
    }
}

pub(crate) fn policy_family_enabled(policy: &DaemonPolicy, action_kind: &str) -> bool {
    policy.enabled_action_families.is_empty()
        || policy
            .enabled_action_families
            .iter()
            .any(|family| policy_family_matches(action_kind, family))
}

pub(crate) fn policy_family_denied(policy: &DaemonPolicy, action_kind: &str) -> bool {
    policy
        .denied_action_families
        .iter()
        .any(|family| policy_family_matches(action_kind, family))
}

pub(crate) fn policy_family_matches(action_kind: &str, family: &str) -> bool {
    action_kind == family
        || action_kind.strip_prefix(family).is_some_and(|suffix| {
            matches!(
                suffix.as_bytes().first(),
                Some(b':') | Some(b'-') | Some(b'_')
            )
        })
}

pub(crate) fn mode_requires_autonomous_workload_family(mode: DaemonMode) -> bool {
    matches!(
        mode,
        DaemonMode::ApplyLowRisk | DaemonMode::ApplyMediumRisk | DaemonMode::ApplyHighRisk
    )
}

pub(crate) fn policy_context_for_input(input: PlannerInput<'_>) -> DaemonPolicyContext {
    DaemonPolicyContext {
        data_quality_ok: !input.observation.data_quality.blocks_action(),
        data_quality_reason_code: input
            .observation
            .data_quality
            .reason_code_strings()
            .first()
            .cloned(),
        system_health_ok: input.system_health.ok_for_apply,
        system_health_reason_code: input.system_health.reason_code.clone(),
        workload_stable: input.observation.workload_identity.is_some(),
        cooldown_active: input
            .controller_state
            .cooldown_until_unix_nanos
            .is_some_and(|until| until > input.observation.now_unix_nanos),
        rollback_pending: input.controller_state.active_experiment.is_some(),
        capabilities: Some(input.capabilities.clone()),
    }
}

pub(crate) fn deny_reason_from_policy(reason_code: &str) -> CandidateDenyReason {
    match reason_code {
        "action_family_not_enabled" => CandidateDenyReason::DisabledFamily,
        "action_family_denied" => CandidateDenyReason::DeniedFamily,
        "safety_class_too_high" => CandidateDenyReason::SafetyClassTooHigh,
        "effect_scope_not_allowed" | "system_wide_action_blocked" => {
            CandidateDenyReason::EffectScopeTooBroad
        }
        "capability_unavailable" => CandidateDenyReason::CapabilityMissing,
        "data_quality_blocked" => CandidateDenyReason::DataQualityLow,
        "system_health_blocked" => CandidateDenyReason::HealthDegraded,
        "explicit_target_required" => CandidateDenyReason::NoExplicitTarget,
        "confidence_too_low" => CandidateDenyReason::ProviderConfidenceTooLow,
        "cooldown_active" => CandidateDenyReason::CooldownActive,
        "high_risk_apply_not_implemented" | "medium_risk_apply_requires_explicit_unlock" => {
            CandidateDenyReason::ManualOnlyHighRisk
        }
        _ => CandidateDenyReason::PolicyRejected,
    }
}
