use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    pub action_id: crate::actions::ActionId,
    pub action_kind: String,
    pub safety_class: crate::actions::SafetyClass,
    pub effect_scope: ActionEffectScope,
    pub rollback: RollbackRequirement,
    pub persistent_effect: bool,
    pub touches_system_wide_state: bool,
    pub requires_explicit_target: bool,
    pub confidence: Option<f32>,
}

#[derive(Clone, Debug)]
pub struct DaemonPolicy {
    pub mode: DaemonMode,
    pub source: ActionSource,
    pub allow_system_wide_actions: bool,
    pub allow_high_risk: bool,
    pub allow_persistent_effects: bool,
    pub min_confidence: f32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PolicyIntent {
    Observe,
    Suggest,
    DryRun,
    Apply,
    Verify,
    Rollback,
}

#[derive(Clone, Debug, PartialEq)]
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
        safety_class: crate::actions::SafetyClass,
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

impl DaemonPolicy {
    pub fn observe(source: ActionSource) -> Self {
        Self {
            mode: DaemonMode::Observe,
            source,
            allow_system_wide_actions: false,
            allow_high_risk: false,
            allow_persistent_effects: false,
            min_confidence: 0.0,
        }
    }

    pub fn suggest(source: ActionSource) -> Self {
        Self {
            mode: DaemonMode::Suggest,
            source,
            allow_system_wide_actions: false,
            allow_high_risk: false,
            allow_persistent_effects: false,
            min_confidence: 0.0,
        }
    }

    pub fn apply_low_risk(source: ActionSource) -> Self {
        Self {
            mode: DaemonMode::ApplyLowRisk,
            source,
            allow_system_wide_actions: false,
            allow_high_risk: false,
            allow_persistent_effects: false,
            min_confidence: crate::autotune::DEFAULT_MIN_FOCUS_CONFIDENCE,
        }
    }

    pub fn apply_medium_risk(source: ActionSource) -> Self {
        Self {
            mode: DaemonMode::ApplyMediumRisk,
            source,
            allow_system_wide_actions: false,
            allow_high_risk: false,
            allow_persistent_effects: false,
            min_confidence: crate::autotune::DEFAULT_MIN_FOCUS_CONFIDENCE,
        }
    }

    pub fn apply_high_risk_explicit(source: ActionSource) -> Self {
        Self {
            mode: DaemonMode::ApplyHighRisk,
            source,
            allow_system_wide_actions: false,
            allow_high_risk: true,
            allow_persistent_effects: false,
            min_confidence: crate::autotune::DEFAULT_MIN_FOCUS_CONFIDENCE,
        }
    }

    pub fn check_action(
        &self,
        intent: PolicyIntent,
        descriptor: &ActionDescriptor,
    ) -> Result<(), PolicyRejection> {
        match intent {
            PolicyIntent::Observe
            | PolicyIntent::DryRun
            | PolicyIntent::Verify
            | PolicyIntent::Rollback => {
                return Ok(());
            }
            PolicyIntent::Suggest => {
                if self.mode == DaemonMode::Observe {
                    return Err(PolicyRejection::IntentNotAllowed {
                        mode: self.mode,
                        intent,
                    });
                }
                return Ok(());
            }
            PolicyIntent::Apply => {}
        }

        match self.mode {
            DaemonMode::Observe | DaemonMode::Suggest => {
                return Err(PolicyRejection::IntentNotAllowed {
                    mode: self.mode,
                    intent,
                });
            }
            DaemonMode::ApplyLowRisk => {
                if descriptor.safety_class != crate::actions::SafetyClass::ReversibleLowRisk {
                    return Err(PolicyRejection::SafetyClassTooHigh {
                        mode: self.mode,
                        safety_class: descriptor.safety_class.clone(),
                    });
                }

                if !descriptor.effect_scope.is_low_risk_apply_scope() {
                    return Err(PolicyRejection::EffectScopeNotAllowed {
                        mode: self.mode,
                        effect_scope: descriptor.effect_scope,
                    });
                }
            }
            DaemonMode::ApplyMediumRisk => {
                if descriptor.safety_class > crate::actions::SafetyClass::ReversibleMediumRisk {
                    return Err(PolicyRejection::SafetyClassTooHigh {
                        mode: self.mode,
                        safety_class: descriptor.safety_class.clone(),
                    });
                }

                if !descriptor.effect_scope.is_explicit_target_scope() {
                    return Err(PolicyRejection::EffectScopeNotAllowed {
                        mode: self.mode,
                        effect_scope: descriptor.effect_scope,
                    });
                }
            }
            DaemonMode::ApplyHighRisk => {
                if descriptor.safety_class == crate::actions::SafetyClass::HighRisk
                    && !self.allow_high_risk
                {
                    return Err(PolicyRejection::HighRiskRequiresExplicitOptIn);
                }
            }
        }

        if descriptor.touches_system_wide_state && !self.allow_system_wide_actions {
            return Err(PolicyRejection::SystemWideActionBlocked);
        }

        if descriptor.persistent_effect && !self.allow_persistent_effects {
            return Err(PolicyRejection::PersistentEffectBlocked);
        }

        match descriptor.rollback {
            RollbackRequirement::RequiredBeforeApply => {}
            RollbackRequirement::Unavailable => {
                return Err(PolicyRejection::RollbackUnavailable);
            }
            RollbackRequirement::NotRequiredForDryRun | RollbackRequirement::BestEffortOnly => {
                return Err(PolicyRejection::RollbackRequired {
                    rollback: descriptor.rollback,
                });
            }
        }

        if descriptor.requires_explicit_target
            && !descriptor.effect_scope.is_explicit_target_scope()
        {
            return Err(PolicyRejection::ExplicitTargetRequired);
        }

        if let Some(confidence) = descriptor.confidence
            && confidence < self.min_confidence
        {
            return Err(PolicyRejection::ConfidenceTooLow {
                confidence,
                min_confidence: self.min_confidence,
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(
        safety_class: crate::actions::SafetyClass,
        effect_scope: ActionEffectScope,
    ) -> ActionDescriptor {
        ActionDescriptor {
            action_id: crate::actions::ActionId("test-action".to_owned()),
            action_kind: "test".to_owned(),
            safety_class,
            effect_scope,
            rollback: RollbackRequirement::RequiredBeforeApply,
            persistent_effect: false,
            touches_system_wide_state: false,
            requires_explicit_target: true,
            confidence: Some(0.90),
        }
    }

    #[test]
    fn daemon_mode_parses_and_formats_kebab_case() {
        assert_eq!(
            "apply-low-risk".parse::<DaemonMode>().unwrap(),
            DaemonMode::ApplyLowRisk
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
    fn observe_allows_observe_dry_run_and_rollback_but_rejects_suggest_and_apply() {
        let policy = DaemonPolicy::observe(ActionSource::Test);
        let desc = descriptor(
            crate::actions::SafetyClass::ReversibleLowRisk,
            ActionEffectScope::LocalProcess,
        );

        assert!(policy.check_action(PolicyIntent::Observe, &desc).is_ok());
        assert!(policy.check_action(PolicyIntent::DryRun, &desc).is_ok());
        assert!(policy.check_action(PolicyIntent::Rollback, &desc).is_ok());
        assert!(matches!(
            policy.check_action(PolicyIntent::Suggest, &desc),
            Err(PolicyRejection::IntentNotAllowed {
                mode: DaemonMode::Observe,
                intent: PolicyIntent::Suggest
            })
        ));
        assert!(matches!(
            policy.check_action(PolicyIntent::Apply, &desc),
            Err(PolicyRejection::IntentNotAllowed {
                mode: DaemonMode::Observe,
                intent: PolicyIntent::Apply
            })
        ));
    }

    #[test]
    fn suggest_allows_suggest_and_dry_run_but_rejects_apply() {
        let policy = DaemonPolicy::suggest(ActionSource::Test);
        let desc = descriptor(
            crate::actions::SafetyClass::HighRisk,
            ActionEffectScope::SystemWide,
        );

        assert!(policy.check_action(PolicyIntent::Suggest, &desc).is_ok());
        assert!(policy.check_action(PolicyIntent::DryRun, &desc).is_ok());
        assert!(matches!(
            policy.check_action(PolicyIntent::Apply, &desc),
            Err(PolicyRejection::IntentNotAllowed {
                mode: DaemonMode::Suggest,
                intent: PolicyIntent::Apply
            })
        ));
    }

    #[test]
    fn apply_low_risk_allows_only_reversible_low_risk_local_process_scope() {
        let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
        let desc = descriptor(
            crate::actions::SafetyClass::ReversibleLowRisk,
            ActionEffectScope::LocalProcess,
        );

        assert!(policy.check_action(PolicyIntent::Apply, &desc).is_ok());

        let medium = descriptor(
            crate::actions::SafetyClass::ReversibleMediumRisk,
            ActionEffectScope::LocalProcess,
        );
        assert!(matches!(
            policy.check_action(PolicyIntent::Apply, &medium),
            Err(PolicyRejection::SafetyClassTooHigh {
                mode: DaemonMode::ApplyLowRisk,
                ..
            })
        ));

        let cgroup = descriptor(
            crate::actions::SafetyClass::ReversibleLowRisk,
            ActionEffectScope::Cgroup,
        );
        assert!(matches!(
            policy.check_action(PolicyIntent::Apply, &cgroup),
            Err(PolicyRejection::EffectScopeNotAllowed {
                mode: DaemonMode::ApplyLowRisk,
                effect_scope: ActionEffectScope::Cgroup
            })
        ));
    }

    #[test]
    fn apply_medium_risk_allows_low_and_medium_risk_target_scopes() {
        let policy = DaemonPolicy::apply_medium_risk(ActionSource::Test);
        let low = descriptor(
            crate::actions::SafetyClass::ReversibleLowRisk,
            ActionEffectScope::LocalProcess,
        );
        let medium = descriptor(
            crate::actions::SafetyClass::ReversibleMediumRisk,
            ActionEffectScope::LocalProcessTree,
        );

        assert!(policy.check_action(PolicyIntent::Apply, &low).is_ok());
        assert!(policy.check_action(PolicyIntent::Apply, &medium).is_ok());

        let high = descriptor(
            crate::actions::SafetyClass::HighRisk,
            ActionEffectScope::LocalProcess,
        );
        assert!(matches!(
            policy.check_action(PolicyIntent::Apply, &high),
            Err(PolicyRejection::SafetyClassTooHigh {
                mode: DaemonMode::ApplyMediumRisk,
                ..
            })
        ));
    }

    #[test]
    fn high_risk_requires_explicit_opt_in() {
        let mut policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
        policy.mode = DaemonMode::ApplyHighRisk;
        policy.allow_high_risk = false;

        let high = descriptor(
            crate::actions::SafetyClass::HighRisk,
            ActionEffectScope::LocalProcess,
        );

        assert!(matches!(
            policy.check_action(PolicyIntent::Apply, &high),
            Err(PolicyRejection::HighRiskRequiresExplicitOptIn)
        ));

        let explicit = DaemonPolicy::apply_high_risk_explicit(ActionSource::Test);
        assert!(explicit.check_action(PolicyIntent::Apply, &high).is_ok());
    }

    #[test]
    fn apply_rejects_system_wide_actions_without_explicit_permission() {
        let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
        let mut desc = descriptor(
            crate::actions::SafetyClass::ReversibleLowRisk,
            ActionEffectScope::LocalProcess,
        );
        desc.touches_system_wide_state = true;

        assert!(matches!(
            policy.check_action(PolicyIntent::Apply, &desc),
            Err(PolicyRejection::SystemWideActionBlocked)
        ));

        let mut allowed_policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
        allowed_policy.allow_system_wide_actions = true;
        assert!(
            allowed_policy
                .check_action(PolicyIntent::Apply, &desc)
                .is_ok()
        );
    }

    #[test]
    fn apply_rejects_persistent_effect_without_explicit_permission() {
        let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
        let mut desc = descriptor(
            crate::actions::SafetyClass::ReversibleLowRisk,
            ActionEffectScope::LocalProcess,
        );
        desc.persistent_effect = true;

        assert!(matches!(
            policy.check_action(PolicyIntent::Apply, &desc),
            Err(PolicyRejection::PersistentEffectBlocked)
        ));

        let mut allowed_policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
        allowed_policy.allow_persistent_effects = true;
        assert!(
            allowed_policy
                .check_action(PolicyIntent::Apply, &desc)
                .is_ok()
        );
    }

    #[test]
    fn apply_rejects_unavailable_or_best_effort_rollback() {
        let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
        let mut desc = descriptor(
            crate::actions::SafetyClass::ReversibleLowRisk,
            ActionEffectScope::LocalProcess,
        );

        desc.rollback = RollbackRequirement::Unavailable;
        assert!(matches!(
            policy.check_action(PolicyIntent::Apply, &desc),
            Err(PolicyRejection::RollbackUnavailable)
        ));

        desc.rollback = RollbackRequirement::BestEffortOnly;
        assert!(matches!(
            policy.check_action(PolicyIntent::Apply, &desc),
            Err(PolicyRejection::RollbackRequired { .. })
        ));
    }

    #[test]
    fn apply_rejects_confidence_below_policy_minimum() {
        let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
        let mut desc = descriptor(
            crate::actions::SafetyClass::ReversibleLowRisk,
            ActionEffectScope::LocalProcess,
        );
        desc.confidence = Some(0.10);

        assert!(matches!(
            policy.check_action(PolicyIntent::Apply, &desc),
            Err(PolicyRejection::ConfidenceTooLow { .. })
        ));
    }
}
