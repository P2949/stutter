use serde::{Deserialize, Serialize};

use crate::autotune::observation::AutotuneObservation;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProtectionDecision {
    Allowed,
    Protected { reason: String },
    RequiresExplicitOptIn { reason: String },
    UnknownDeny { reason: String },
}

impl ProtectionDecision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed)
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Allowed => None,
            Self::Protected { reason }
            | Self::RequiresExplicitOptIn { reason }
            | Self::UnknownDeny { reason } => Some(reason),
        }
    }
}

pub fn mutation_allowed_for_pid(
    pid: u32,
    candidate_family: &str,
    observation: &AutotuneObservation,
) -> ProtectionDecision {
    if pid == 0 {
        return ProtectionDecision::UnknownDeny {
            reason: format!("{candidate_family} candidate has no explicit nonzero target"),
        };
    }

    if observation.focus_confidence < crate::autotune::DEFAULT_MIN_FOCUS_CONFIDENCE {
        return ProtectionDecision::UnknownDeny {
            reason: format!(
                "{candidate_family} candidate target denied because focus confidence is low"
            ),
        };
    }

    if let Some(task) = observation
        .protected_tasks
        .iter()
        .find(|task| task.tid == pid || task.process_pid == pid)
    {
        return ProtectionDecision::Protected {
            reason: format!(
                "{candidate_family} candidate target pid={} comm={} is protected: {}",
                task.process_pid, task.comm, task.reason
            ),
        };
    }

    if observation.focus_has_critical_realtime_warning() {
        return ProtectionDecision::Protected {
            reason: format!(
                "{candidate_family} candidate denied because focus contains critical realtime warning"
            ),
        };
    }

    ProtectionDecision::Allowed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        autotune::observation::ProtectedTask, focus::FocusGroupKind, process_tree::TaskClass,
    };

    #[test]
    fn protected_pid_is_denied() {
        let observation = AutotuneObservation {
            focus_kind: Some(FocusGroupKind::Game),
            focus_confidence: 0.95,
            protected_tasks: vec![ProtectedTask {
                tid: 55,
                process_pid: 55,
                comm: "pipewire".to_owned(),
                class: TaskClass::AudioRealtime,
                reason: "audio realtime task".to_owned(),
            }],
            ..AutotuneObservation::default()
        };

        let decision = mutation_allowed_for_pid(55, "nice", &observation);

        assert!(!decision.is_allowed());
        assert!(decision.reason().unwrap().contains("protected"));
    }

    #[test]
    fn confident_unprotected_pid_is_allowed() {
        let observation = AutotuneObservation {
            focus_kind: Some(FocusGroupKind::Compile),
            focus_confidence: 0.95,
            ..AutotuneObservation::default()
        };

        assert!(mutation_allowed_for_pid(1234, "uclamp", &observation).is_allowed());
    }
}
