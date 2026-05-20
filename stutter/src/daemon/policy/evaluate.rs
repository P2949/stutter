//! Daemon policy evaluation and explanation; this module must not own config construction.

use super::{
    ActionDescriptor, ActionEffectScope, ActionSource, DaemonMode, DaemonPolicy,
    DaemonPolicyContext, DaemonPolicyVerdict, HIGH_RISK_APPLY_IMPLEMENTED, PolicyIntent,
    PolicyRejection, RollbackRequirement,
    capability::{action_kind_matches_family, unavailable_capability_for_action},
    remote::{record_remote_apply_rules, record_remote_target_count_rule},
};
use crate::{
    actions::SafetyClass,
    daemon::explain::{PolicyDecisionKind, PolicyExplanation, PolicyRuleEvaluation},
    error::DaemonPolicyError,
};

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

        let medium_risk_system_scope_allowed = self.medium_risk_system_scope_allowed(descriptor);
        let passed = !descriptor.touches_system_wide_state
            || self.allow_system_wide_apply
            || medium_risk_system_scope_allowed;
        record_policy_rule(
            &mut evaluated_rules,
            &mut first_rejection,
            "system_wide_apply_permission",
            passed,
            if passed {
                if medium_risk_system_scope_allowed && !self.allow_system_wide_apply {
                    format!(
                        "system-wide apply permission is satisfied for medium-risk scope {:?}",
                        descriptor.effect_scope
                    )
                } else {
                    "system-wide apply permission is satisfied".to_owned()
                }
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

    fn medium_risk_system_scope_allowed(&self, descriptor: &ActionDescriptor) -> bool {
        if self.mode != DaemonMode::ApplyMediumRisk
            || descriptor.safety_class > SafetyClass::ReversibleMediumRisk
        {
            return false;
        }

        match descriptor.effect_scope {
            ActionEffectScope::Irq => true,
            ActionEffectScope::CpuPower | ActionEffectScope::GpuPower => {
                self.allow_gpu_power_in_autotune
            }
            ActionEffectScope::VmKnob => self.allow_vm_knobs_in_autotune,
            _ => false,
        }
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

pub(in crate::daemon::policy) fn record_policy_rule(
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
