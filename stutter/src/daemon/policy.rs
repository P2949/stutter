use std::{collections::BTreeSet, fmt, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::{
    actions::{ActionId, SafetyClass},
    daemon::{
        config::DaemonConfig,
        explain::{PolicyDecisionKind, PolicyExplanation, PolicyRuleEvaluation},
    },
};

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
        matches!(self, Self::LocalProcess | Self::LocalProcessTree)
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteApplyPolicy {
    pub allow_remote_apply: bool,
    pub require_loopback_bind: bool,
    pub require_auth: bool,
    pub max_remote_targets: usize,
}

impl Default for RemoteApplyPolicy {
    fn default() -> Self {
        Self {
            allow_remote_apply: false,
            require_loopback_bind: true,
            require_auth: true,
            max_remote_targets: 1,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DaemonPolicy {
    pub mode: DaemonMode,
    pub source: ActionSource,
    pub max_safety_class: SafetyClass,
    pub allowed_effect_scopes: BTreeSet<ActionEffectScope>,
    pub denied_action_families: BTreeSet<String>,
    pub rollback_required_before_apply: bool,
    pub allow_system_wide_actions: bool,
    pub allow_high_risk: bool,
    pub allow_persistent_effects: bool,
    pub min_confidence: f32,
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
    ExplicitTargetRequired,
    HighRiskRequiresExplicitOptIn,
    ConfidenceTooLow {
        confidence: f32,
        min_confidence: f32,
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
            Self::ExplicitTargetRequired => f.write_str("apply requires an explicit target scope"),
            Self::HighRiskRequiresExplicitOptIn => {
                f.write_str("high-risk apply requires explicit high-risk opt-in")
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
        }
    }
}

impl std::error::Error for PolicyRejection {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemotePolicyContext {
    pub is_loopback_bind: bool,
    pub auth_configured: bool,
    pub valid_auth: bool,
    pub max_mode: DaemonMode,
    pub max_safety_class: SafetyClass,
    pub allow_high_risk: bool,
    pub allow_system_wide_actions: bool,
    pub max_remote_targets: usize,
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
        && context.max_safety_class < max_safety_class
    {
        max_safety_class = context.max_safety_class.clone();
    }

    let mut allow_system_wide_actions = config.safety.allow_system_wide_actions;
    let mut allow_high_risk =
        config.mode == DaemonMode::ApplyHighRisk && config.safety.allow_high_risk;

    if let Some(context) = remote_context {
        allow_system_wide_actions &= context.allow_system_wide_actions;
        allow_high_risk &= context.allow_high_risk;
    }

    DaemonPolicy {
        mode: config.mode,
        source: config.source,
        max_safety_class,
        allowed_effect_scopes: allowed_effect_scopes_for_mode(config.mode),
        denied_action_families: config.safety.denied_action_families.clone(),
        rollback_required_before_apply: config.mode.supports_apply(),
        allow_system_wide_actions,
        allow_high_risk,
        allow_persistent_effects: config.safety.allow_persistent_effects,
        min_confidence: min_confidence_for_config(config),
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
        DaemonMode::ApplyLowRisk | DaemonMode::ApplyMediumRisk => BTreeSet::from([
            ActionEffectScope::LocalProcess,
            ActionEffectScope::LocalProcessTree,
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

fn remote_apply_policy_for_config(
    config: &DaemonConfig,
    remote_context: Option<&RemotePolicyContext>,
) -> RemoteApplyPolicy {
    let Some(context) = remote_context else {
        return RemoteApplyPolicy::default();
    };

    let mode_supported_by_limits = remote_mode_supported_by_context(config.mode, context);
    let auth_allowed =
        !config.remote.require_auth_for_apply || (context.auth_configured && context.valid_auth);
    let bind_allowed = config.remote.allow_non_loopback_apply || context.is_loopback_bind;

    RemoteApplyPolicy {
        allow_remote_apply: config.remote.allow_remote_apply
            && config.mode.supports_apply()
            && mode_supported_by_limits
            && auth_allowed
            && bind_allowed,
        require_loopback_bind: config.mode.supports_apply()
            && !config.remote.allow_non_loopback_apply,
        require_auth: config.mode.supports_apply() && config.remote.require_auth_for_apply,
        max_remote_targets: context.max_remote_targets,
    }
}

fn remote_mode_supported_by_context(mode: DaemonMode, context: &RemotePolicyContext) -> bool {
    let mode_max_safety_class = max_safety_class_for_mode(mode);

    mode <= context.max_mode
        && mode_max_safety_class <= context.max_safety_class
        && (mode != DaemonMode::ApplyHighRisk || context.allow_high_risk)
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
    config.safety.min_confidence = min_confidence_for_config(&config);
    config
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

    pub fn explain_action(
        &self,
        intent: PolicyIntent,
        descriptor: &ActionDescriptor,
    ) -> PolicyExplanation {
        let mut evaluated_rules = Vec::new();
        let mut first_rejection = None;

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
            }
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

        let passed = !descriptor.touches_system_wide_state || self.allow_system_wide_actions;
        record_policy_rule(
            &mut evaluated_rules,
            &mut first_rejection,
            "system_wide_permission",
            passed,
            if passed {
                "system-wide action permission is satisfied".to_owned()
            } else {
                "system-wide action is blocked by daemon policy".to_owned()
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
        match self.explain_action(intent, descriptor).decision {
            PolicyDecisionKind::Allowed => Ok(()),
            PolicyDecisionKind::Rejected { rejection } => Err(rejection),
        }
    }

    fn finish_policy_explanation(
        &self,
        intent: PolicyIntent,
        descriptor: &ActionDescriptor,
        evaluated_rules: Vec<PolicyRuleEvaluation>,
        rejection: Option<PolicyRejection>,
    ) -> PolicyExplanation {
        let (decision, final_reason) = match rejection {
            Some(rejection) => {
                let final_reason = rejection.to_string();
                (PolicyDecisionKind::Rejected { rejection }, final_reason)
            }
            None => (
                PolicyDecisionKind::Allowed,
                "action is allowed by daemon policy".to_owned(),
            ),
        };

        PolicyExplanation {
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
        assert!(!policy.allow_system_wide_actions);
        assert!(!policy.allow_high_risk);
        assert!(!policy.allow_persistent_effects);
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
    fn suggest_allows_suggestion_without_apply() {
        let policy = DaemonPolicy::suggest(ActionSource::Test);
        let desc = descriptor_with(
            SafetyClass::HighRisk,
            ActionEffectScope::SystemWide,
            RollbackRequirement::Unavailable,
        );

        assert!(policy.check_action(PolicyIntent::Suggest, &desc).is_ok());
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
    fn apply_high_risk_rejects_without_explicit_high_risk_unlock() {
        let mut policy = DaemonPolicy::apply_high_risk_explicit(ActionSource::Test);
        policy.allow_high_risk = false;
        let high = descriptor(SafetyClass::HighRisk);

        assert!(matches!(
            policy.check_action(PolicyIntent::Apply, &high),
            Err(PolicyRejection::HighRiskRequiresExplicitOptIn)
        ));

        let explicit = DaemonPolicy::apply_high_risk_explicit(ActionSource::Test);
        assert!(explicit.check_action(PolicyIntent::Apply, &high).is_ok());
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
        config.safety.allow_system_wide_actions = true;
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
        assert!(first.allow_system_wide_actions);
        assert!(first.allow_persistent_effects);
        assert!(first.denied_action_families.contains("gpu-power"));
    }

    #[test]
    fn build_daemon_policy_remote_context_is_deterministic_and_respects_limits() {
        let mut config = crate::daemon::config::DaemonConfig {
            mode: DaemonMode::ApplyLowRisk,
            source: ActionSource::RemoteAgent,
            ..crate::daemon::config::DaemonConfig::default()
        };
        config.remote.allow_remote_apply = true;
        config.safety.allow_system_wide_actions = true;

        let remote_context = RemotePolicyContext {
            is_loopback_bind: true,
            auth_configured: true,
            valid_auth: true,
            max_mode: DaemonMode::ApplyLowRisk,
            max_safety_class: SafetyClass::ReversibleLowRisk,
            allow_high_risk: false,
            allow_system_wide_actions: false,
            max_remote_targets: 3,
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
        assert!(!first.allow_system_wide_actions);
    }
}
