use super::{
    super::{
        ActionDescriptor, DaemonPolicy, DaemonPolicyContext, PolicyIntent, PolicyRejection,
        capability::unavailable_capability_for_action,
    },
    record_policy_rule,
};
use crate::daemon::explain::PolicyRuleEvaluation;

pub(super) fn record_context_rules(
    _policy: &DaemonPolicy,
    intent: &PolicyIntent,
    descriptor: &ActionDescriptor,
    context: &DaemonPolicyContext,
    evaluated_rules: &mut Vec<PolicyRuleEvaluation>,
    first_rejection: &mut Option<PolicyRejection>,
) {
    let apply_intent = matches!(intent, PolicyIntent::Apply);
    let data_quality_reason = match &context.data_quality_reason_code {
        Some(reason) => reason.clone(),
        None => "insufficient_data".to_owned(),
    };
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

    let health_reason = match &context.system_health_reason_code {
        Some(reason) => reason.clone(),
        None => "system_health_degraded".to_owned(),
    };
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
    let capability_rejection =
        missing_capability.map(|feature| PolicyRejection::CapabilityUnavailable {
            action_kind: descriptor.action_kind.clone(),
            feature,
        });
    let rejection = if capability_passed {
        None
    } else {
        capability_rejection
    };
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
        rejection,
    );
}
