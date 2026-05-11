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
    use crate::actions::{ActionId, SafetyClass};

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
}
