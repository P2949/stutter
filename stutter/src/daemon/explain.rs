use serde::{Deserialize, Serialize};

use crate::{
    actions::{ActionId, SafetyClass},
    daemon::{
        capabilities::DaemonCapabilities,
        health::SystemHealthSnapshot,
        policy::{
            ActionDescriptor, ActionEffectScope, ActionSource, DaemonMode, DaemonPolicy,
            DaemonPolicyContext, DaemonPolicyVerdict, PolicyIntent, PolicyRejection,
            RollbackRequirement,
        },
        state::{DaemonPhase, DaemonState},
        watchdog::DaemonWatchdogReport,
    },
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PolicyExplainLine {
    pub rule: String,
    pub outcome: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonPolicyExplanation {
    pub mode: DaemonMode,
    pub source: ActionSource,
    pub lines: Vec<PolicyExplainLine>,
}

impl DaemonPolicyExplanation {
    pub fn from_policy(policy: &DaemonPolicy) -> Self {
        Self::from_policy_with_context(policy, &DaemonPolicyContext::default())
    }

    pub fn from_policy_with_context(policy: &DaemonPolicy, context: &DaemonPolicyContext) -> Self {
        let mut lines = vec![PolicyExplainLine {
            rule: "policy_model".to_owned(),
            outcome: "available".to_owned(),
            reason: format!(
                "daemon policy model is available for mode={} source={:?}",
                policy.mode, policy.source
            ),
        }];

        for case in canonical_policy_explain_cases() {
            let explanation =
                policy.explain_action_with_context(case.intent.clone(), &case.descriptor, context);
            let outcome = match &explanation.decision {
                PolicyDecisionKind::Allowed => "allowed".to_owned(),
                PolicyDecisionKind::Rejected { rejection } => {
                    format!("rejected:{}", rejection.reason_code())
                }
            };
            lines.push(PolicyExplainLine {
                rule: format!("action:{}", case.name),
                outcome,
                reason: explanation.final_reason.clone(),
            });
            for rule in explanation.evaluated_rules {
                lines.push(PolicyExplainLine {
                    rule: format!("action:{}:{}", case.name, rule.rule),
                    outcome: if rule.passed { "passed" } else { "failed" }.to_owned(),
                    reason: rule.reason,
                });
            }
        }

        Self {
            mode: policy.mode,
            source: policy.source,
            lines,
        }
    }
}

pub fn policy_context_from_daemon_status(
    state: &DaemonState,
    health: &SystemHealthSnapshot,
    capabilities: &DaemonCapabilities,
) -> DaemonPolicyContext {
    policy_context_from_daemon_status_at(
        state,
        health,
        capabilities,
        crate::audit::unix_nanos_now(),
    )
}

pub fn policy_context_from_daemon_status_at(
    state: &DaemonState,
    health: &SystemHealthSnapshot,
    capabilities: &DaemonCapabilities,
    now_unix_nanos: u128,
) -> DaemonPolicyContext {
    let mut context = DaemonPolicyContext::default().with_system_health(health);
    context.capabilities = Some(capabilities.clone());
    context.cooldown_active = state
        .cooldown_until_unix_nanos
        .is_some_and(|cooldown_until| cooldown_until > now_unix_nanos);
    context.rollback_pending = state.active_rollback.is_some();

    for degraded in &state.degraded {
        match degraded.category.as_str() {
            "data_quality" => {
                context.data_quality_ok = false;
                context.data_quality_reason_code = Some(degraded.message.clone());
            }
            "workload_identity" | "workload_stability" | "target_identity" => {
                context.workload_stable = false;
            }
            _ => {}
        }
    }

    context
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonStatusExplanation {
    pub why_no_optimize: Vec<String>,
    pub what_changed: Vec<String>,
}

impl DaemonStatusExplanation {
    pub fn from_state_health_watchdog(
        state: &DaemonState,
        health: &SystemHealthSnapshot,
        watchdog: &DaemonWatchdogReport,
    ) -> Self {
        Self::from_state_health_watchdog_at(state, health, watchdog, crate::audit::unix_nanos_now())
    }

    pub fn from_state_health_watchdog_at(
        state: &DaemonState,
        health: &SystemHealthSnapshot,
        watchdog: &DaemonWatchdogReport,
        now_unix_nanos: u128,
    ) -> Self {
        Self {
            why_no_optimize: daemon_no_optimize_reasons(state, health, watchdog, now_unix_nanos),
            what_changed: daemon_change_summary(state, health, watchdog),
        }
    }
}

fn daemon_no_optimize_reasons(
    state: &DaemonState,
    health: &SystemHealthSnapshot,
    watchdog: &DaemonWatchdogReport,
    now_unix_nanos: u128,
) -> Vec<String> {
    let mut reasons = Vec::new();

    if !state.mode.supports_apply() {
        reasons.push("observe_only_mode".to_owned());
    }
    if matches!(state.phase, DaemonPhase::Paused) {
        reasons.push("daemon_paused".to_owned());
    }
    if let Some(fault) = state.faulted.as_ref() {
        reasons.push(format!("faulted:{}", fault.reason));
    }
    if let Some(rollback) = state.active_rollback.as_ref() {
        reasons.push(format!("rollback_pending:{}", rollback.action_id));
    }
    if let Some(cooldown_until) = state
        .cooldown_until_unix_nanos
        .filter(|cooldown_until| *cooldown_until > now_unix_nanos)
    {
        reasons.push(format!("cooldown_active_until_unix_nanos:{cooldown_until}"));
    }
    if !health.ok_for_apply {
        reasons.push(format!(
            "health_blocked:{}",
            health.reason_code.as_deref().unwrap_or("unknown")
        ));
    }
    for issue in &watchdog.issues {
        reasons.push(format!("watchdog:{}:{}", issue.reason_code, issue.message));
    }
    for degraded in &state.degraded {
        reasons.push(format!(
            "degraded:{}:{}",
            degraded.category, degraded.message
        ));
    }
    if let Some(decision) = state.last_decision.as_ref()
        && decision.decision != "candidate_applied"
    {
        reasons.push(format!(
            "last_decision:{}:{}",
            decision.decision, decision.reason
        ));
    }

    if reasons.is_empty() {
        reasons.push("no_blocking_reason_captured".to_owned());
    }

    reasons
}

fn daemon_change_summary(
    state: &DaemonState,
    health: &SystemHealthSnapshot,
    watchdog: &DaemonWatchdogReport,
) -> Vec<String> {
    let mut changes = Vec::new();

    changes.push(format!("phase:{}", state.phase.lifecycle_label()));
    changes.push(format!("mode:{}", state.mode.as_str()));
    changes.push(format!("health:{}", health.state.as_str()));
    changes.push(format!("watchdog_ok:{}", watchdog.ok));

    if let Some(target) = state.active_target.as_ref() {
        changes.push(format!(
            "active_workload:root_pid={} comm={} targets={}",
            target
                .root_pid
                .map(|pid| pid.to_string())
                .unwrap_or_else(|| "unknown".to_owned()),
            target.comm.as_deref().unwrap_or("unknown"),
            target.active_targets
        ));
    }
    if let Some(experiment) = state.active_experiment.as_ref() {
        changes.push(format!(
            "active_candidate:{}:{}",
            experiment.action_id,
            experiment.candidate_name.as_deref().unwrap_or("unknown")
        ));
    }
    if let Some(rollback) = state.active_rollback.as_ref() {
        changes.push(format!(
            "rollback:{}:{}",
            rollback.action_id, rollback.rollback_available
        ));
    }
    if let Some(cooldown) = state.cooldown_until_unix_nanos {
        changes.push(format!("cooldown_until_unix_nanos:{cooldown}"));
    }
    if let Some(decision) = state.last_decision.as_ref() {
        changes.push(format!("last_decision:{}", decision.decision));
    }
    if !state.profile_memory.profiles.is_empty() {
        changes.push(format!(
            "profile_memory_count:{}",
            state.profile_memory.profiles.len()
        ));
    }

    changes
}

struct CanonicalPolicyExplainCase {
    name: &'static str,
    intent: PolicyIntent,
    descriptor: ActionDescriptor,
}

fn canonical_policy_explain_cases() -> Vec<CanonicalPolicyExplainCase> {
    vec![
        CanonicalPolicyExplainCase {
            name: "observe_status",
            intent: PolicyIntent::Observe,
            descriptor: descriptor(
                "daemon-status",
                SafetyClass::ObserveOnly,
                ActionEffectScope::ObserveOnly,
                RollbackRequirement::NotRequiredForDryRun,
            ),
        },
        CanonicalPolicyExplainCase {
            name: "apply_low_risk_cpu_affinity",
            intent: PolicyIntent::Apply,
            descriptor: descriptor(
                "cpu_affinity_profile",
                SafetyClass::ReversibleLowRisk,
                ActionEffectScope::LocalProcessTree,
                RollbackRequirement::RequiredBeforeApply,
            ),
        },
        CanonicalPolicyExplainCase {
            name: "apply_medium_risk_nice",
            intent: PolicyIntent::Apply,
            descriptor: descriptor(
                "nice",
                SafetyClass::ReversibleMediumRisk,
                ActionEffectScope::LocalProcessTree,
                RollbackRequirement::RequiredBeforeApply,
            ),
        },
        CanonicalPolicyExplainCase {
            name: "apply_high_risk_sysfs",
            intent: PolicyIntent::Apply,
            descriptor: descriptor(
                "sysfs",
                SafetyClass::HighRisk,
                ActionEffectScope::Sysfs,
                RollbackRequirement::RequiredBeforeApply,
            )
            .with_system_wide_effect(),
        },
        CanonicalPolicyExplainCase {
            name: "apply_without_rollback",
            intent: PolicyIntent::Apply,
            descriptor: descriptor(
                "cpu_affinity_profile",
                SafetyClass::ReversibleLowRisk,
                ActionEffectScope::LocalProcessTree,
                RollbackRequirement::Unavailable,
            ),
        },
    ]
}

fn descriptor(
    action_kind: &'static str,
    safety_class: SafetyClass,
    effect_scope: ActionEffectScope,
    rollback: RollbackRequirement,
) -> ActionDescriptor {
    ActionDescriptor {
        action_id: ActionId::new(format!("explain:{action_kind}")),
        action_kind: action_kind.to_owned(),
        safety_class,
        effect_scope,
        rollback,
        persistent_effect: false,
        touches_system_wide_state: false,
        requires_explicit_target: matches!(
            effect_scope,
            ActionEffectScope::LocalProcess | ActionEffectScope::LocalProcessTree
        ),
        confidence: Some(1.0),
    }
}

trait ExplainDescriptorExt {
    fn with_system_wide_effect(self) -> Self;
}

impl ExplainDescriptorExt for ActionDescriptor {
    fn with_system_wide_effect(mut self) -> Self {
        self.touches_system_wide_state = true;
        self
    }
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct PolicyExplanation {
    pub verdict: DaemonPolicyVerdict,
    pub decision: PolicyDecisionKind,
    pub intent: PolicyIntent,
    pub action_id: ActionId,
    pub action_kind: String,
    pub mode: DaemonMode,
    pub source: ActionSource,
    pub evaluated_rules: Vec<PolicyRuleEvaluation>,
    pub final_reason: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PolicyDecisionKind {
    Allowed,
    Rejected { rejection: PolicyRejection },
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct PolicyRuleEvaluation {
    pub rule: &'static str,
    pub passed: bool,
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        actions::ActionId,
        daemon::{
            capabilities::DaemonCapabilities,
            health::SystemHealthSnapshot,
            policy::{ActionSource, DaemonPolicy, PolicyIntent},
            state::{DaemonDecisionState, DaemonDegradedStatus, DaemonTargetState},
            watchdog::{DaemonWatchdogConfig, DaemonWatchdogInputs, evaluate_daemon_watchdog},
        },
    };

    #[test]
    fn policy_explanation_can_be_rendered_from_policy() {
        let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);

        let explanation = DaemonPolicyExplanation::from_policy(&policy);

        assert_eq!(explanation.mode, policy.mode);
        assert_eq!(explanation.source, policy.source);
        assert_eq!(explanation.lines[0].rule, "policy_model");
        assert!(
            explanation
                .lines
                .iter()
                .all(|line| !line.reason.contains("later patch"))
        );
        assert!(explanation.lines.iter().any(|line| {
            line.rule == "action:apply_low_risk_cpu_affinity" && line.outcome == "allowed"
        }));
        assert!(explanation.lines.iter().any(|line| {
            line.rule == "action:apply_without_rollback"
                && line.outcome == "rejected:rollback_unavailable"
        }));
        assert!(explanation.lines.iter().any(|line| {
            line.rule == "action:apply_without_rollback:rollback_available"
                && line.outcome == "failed"
        }));
    }

    #[test]
    fn policy_explanation_with_status_context_reports_live_safety_gates() {
        let mut state = DaemonState {
            mode: DaemonMode::ApplyLowRisk,
            cooldown_until_unix_nanos: Some(crate::audit::unix_nanos_now() + 3_600_000_000_000),
            active_rollback: Some(crate::daemon::state::DaemonRollbackState {
                action_id: crate::actions::ActionId::new("action-1"),
                mode: DaemonMode::ApplyLowRisk,
                safety_class: crate::actions::SafetyClass::ReversibleLowRisk,
                rollback_available: true,
                token: None,
                manual_restore_command: Some("stutter daemon emergency-restore".to_owned()),
            }),
            ..DaemonState::default()
        };
        state.degraded.push(DaemonDegradedStatus {
            category: "data_quality".to_owned(),
            message: "insufficient_samples".to_owned(),
        });
        let health = SystemHealthSnapshot {
            ok_for_apply: false,
            reason_code: Some("cpu_overheated".to_owned()),
            ..SystemHealthSnapshot::default()
        };
        let capabilities = DaemonCapabilities {
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
            privileged_worker_socket_reachable: Some(true),
        };
        let context = policy_context_from_daemon_status(&state, &health, &capabilities);
        let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);

        let explanation = DaemonPolicyExplanation::from_policy_with_context(&policy, &context);

        assert!(explanation.lines.iter().any(|line| {
            line.rule == "action:apply_low_risk_cpu_affinity:data_quality_gate"
                && line.outcome == "failed"
                && line.reason.contains("insufficient_samples")
        }));
        assert!(explanation.lines.iter().any(|line| {
            line.rule == "action:apply_low_risk_cpu_affinity:system_health_gate"
                && line.outcome == "failed"
                && line.reason.contains("cpu_overheated")
        }));
        assert!(explanation.lines.iter().any(|line| {
            line.rule == "action:apply_low_risk_cpu_affinity:cooldown_gate"
                && line.outcome == "failed"
        }));
        assert!(explanation.lines.iter().any(|line| {
            line.rule == "action:apply_low_risk_cpu_affinity:rollback_pending_gate"
                && line.outcome == "failed"
        }));
    }

    #[test]
    fn policy_context_only_treats_future_cooldowns_as_active() {
        let state = DaemonState {
            mode: DaemonMode::ApplyLowRisk,
            cooldown_until_unix_nanos: Some(100),
            ..DaemonState::default()
        };
        let health = SystemHealthSnapshot::default();
        let capabilities = DaemonCapabilities {
            kernel_release: None,
            btf_available: true,
            sched_tracepoints_available: true,
            perf_permissions_likely: true,
            perf_event_paranoid: None,
            cgroup_v2_available: true,
            sched_ext_available: true,
            uclamp_available: true,
            ionice_available: true,
            irq_affinity_available: true,
            gpu_sysfs_available: true,
            privileged_worker_socket_reachable: Some(true),
        };

        let active_context =
            policy_context_from_daemon_status_at(&state, &health, &capabilities, 99);
        let expired_context =
            policy_context_from_daemon_status_at(&state, &health, &capabilities, 100);

        assert!(active_context.cooldown_active);
        assert!(!expired_context.cooldown_active);
    }

    #[test]
    fn status_explanation_only_reports_future_cooldowns() {
        let state = DaemonState {
            mode: DaemonMode::ApplyLowRisk,
            cooldown_until_unix_nanos: Some(100),
            ..DaemonState::default()
        };
        let health = SystemHealthSnapshot::default();
        let watchdog = evaluate_daemon_watchdog(
            DaemonWatchdogInputs::from_state_and_health(&state, &health),
            &DaemonWatchdogConfig::default(),
        );

        let active =
            DaemonStatusExplanation::from_state_health_watchdog_at(&state, &health, &watchdog, 99);
        let expired =
            DaemonStatusExplanation::from_state_health_watchdog_at(&state, &health, &watchdog, 100);

        assert!(
            active
                .why_no_optimize
                .iter()
                .any(|reason| reason == "cooldown_active_until_unix_nanos:100")
        );
        assert!(
            expired
                .why_no_optimize
                .iter()
                .all(|reason| !reason.starts_with("cooldown_active_until_unix_nanos"))
        );
    }

    #[test]
    fn policy_explanation_from_observe_policy_rejects_apply_cases() {
        let policy = DaemonPolicy::observe(ActionSource::Test);

        let explanation = DaemonPolicyExplanation::from_policy(&policy);

        assert!(explanation.lines.iter().any(|line| {
            line.rule == "action:apply_low_risk_cpu_affinity"
                && line.outcome == "rejected:intent_not_allowed"
        }));
        assert!(
            explanation
                .lines
                .iter()
                .any(|line| { line.rule == "action:observe_status" && line.outcome == "allowed" })
        );
    }

    #[test]
    fn status_explanation_reports_no_optimize_reasons_and_changes() {
        let state = DaemonState {
            mode: DaemonMode::Observe,
            phase: DaemonPhase::Paused,
            active_target: Some(DaemonTargetState {
                root_pid: Some(1234),
                active_targets: 2,
                comm: Some("game".to_owned()),
            }),
            degraded: vec![DaemonDegradedStatus {
                category: "data_quality".to_owned(),
                message: "insufficient samples".to_owned(),
            }],
            last_decision: Some(DaemonDecisionState {
                decision: "noop".to_owned(),
                reason: "insufficient data".to_owned(),
                unix_nanos: Some(1),
                diagnostic_current_raw_score_total: None,
                candidate_count: None,
                top_denied_reason: None,
                planner: None,
                situation: None,
                focus_kind: None,
            }),
            ..DaemonState::default()
        };
        let health = SystemHealthSnapshot::default();
        let watchdog = evaluate_daemon_watchdog(
            DaemonWatchdogInputs::from_state_and_health(&state, &health),
            &DaemonWatchdogConfig::default(),
        );

        let explanation =
            DaemonStatusExplanation::from_state_health_watchdog(&state, &health, &watchdog);

        assert!(
            explanation
                .why_no_optimize
                .iter()
                .any(|reason| reason == "observe_only_mode")
        );
        assert!(
            explanation
                .why_no_optimize
                .iter()
                .any(|reason| reason == "daemon_paused")
        );
        assert!(
            explanation
                .why_no_optimize
                .iter()
                .any(|reason| reason.contains("insufficient samples"))
        );
        assert!(
            explanation
                .what_changed
                .iter()
                .any(|change| change == "phase:paused")
        );
        assert!(
            explanation
                .what_changed
                .iter()
                .any(|change| change.contains("active_workload:root_pid=1234"))
        );
    }

    #[test]
    fn structured_policy_explanation_serializes() {
        let explanation = PolicyExplanation {
            verdict: DaemonPolicyVerdict::Allow,
            decision: PolicyDecisionKind::Allowed,
            intent: PolicyIntent::Apply,
            action_id: ActionId::new("test-action".to_owned()),
            action_kind: "test".to_owned(),
            mode: DaemonMode::ApplyLowRisk,
            source: ActionSource::Test,
            evaluated_rules: vec![PolicyRuleEvaluation {
                rule: "intent_allowed",
                passed: true,
                reason: "apply intent is allowed in daemon mode apply-low-risk".to_owned(),
            }],
            final_reason: "action is allowed by daemon policy".to_owned(),
        };

        let json = serde_json::to_string(&explanation).unwrap();

        assert!(json.contains("\"decision\""));
        assert!(json.contains("\"verdict\":\"allow\""));
        assert!(json.contains("\"evaluated_rules\""));
        assert!(json.contains("intent_allowed"));
    }
}
