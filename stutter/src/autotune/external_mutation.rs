use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ExternalMutationPolicy {
    #[default]
    Fault,
    Restore,
    Resync,
}

impl ExternalMutationPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fault => "fault",
            Self::Restore => "restore",
            Self::Resync => "resync",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExternalMutationRecoveryDecision {
    RestoreExpectedState,
    AcceptExternalMutationAndResync,
    AbandonKeptAction,
    FaultRequireManualRestore,
}

impl ExternalMutationRecoveryDecision {
    pub fn reason_code(self) -> &'static str {
        match self {
            Self::RestoreExpectedState => "restore_expected_state",
            Self::AcceptExternalMutationAndResync => "accept_external_mutation_and_resync",
            Self::AbandonKeptAction => "abandon_kept_action",
            Self::FaultRequireManualRestore => "fault_require_manual_restore",
        }
    }
}

pub fn recovery_decision_for_active_experiment(
    policy: ExternalMutationPolicy,
) -> ExternalMutationRecoveryDecision {
    match policy {
        ExternalMutationPolicy::Restore => ExternalMutationRecoveryDecision::RestoreExpectedState,
        ExternalMutationPolicy::Resync => {
            ExternalMutationRecoveryDecision::AcceptExternalMutationAndResync
        }
        ExternalMutationPolicy::Fault => {
            ExternalMutationRecoveryDecision::FaultRequireManualRestore
        }
    }
}

pub fn recovery_decision_for_kept_action(
    policy: ExternalMutationPolicy,
) -> ExternalMutationRecoveryDecision {
    match policy {
        ExternalMutationPolicy::Resync => ExternalMutationRecoveryDecision::AbandonKeptAction,
        ExternalMutationPolicy::Fault | ExternalMutationPolicy::Restore => {
            ExternalMutationRecoveryDecision::FaultRequireManualRestore
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_experiment_policy_maps_to_recovery_decisions() {
        assert_eq!(
            recovery_decision_for_active_experiment(ExternalMutationPolicy::Fault),
            ExternalMutationRecoveryDecision::FaultRequireManualRestore
        );
        assert_eq!(
            recovery_decision_for_active_experiment(ExternalMutationPolicy::Restore),
            ExternalMutationRecoveryDecision::RestoreExpectedState
        );
        assert_eq!(
            recovery_decision_for_active_experiment(ExternalMutationPolicy::Resync),
            ExternalMutationRecoveryDecision::AcceptExternalMutationAndResync
        );
    }

    #[test]
    fn kept_action_restore_remains_manual_until_rollback_executor_is_available() {
        assert_eq!(
            recovery_decision_for_kept_action(ExternalMutationPolicy::Restore),
            ExternalMutationRecoveryDecision::FaultRequireManualRestore
        );
        assert_eq!(
            recovery_decision_for_kept_action(ExternalMutationPolicy::Resync),
            ExternalMutationRecoveryDecision::AbandonKeptAction
        );
    }
}
