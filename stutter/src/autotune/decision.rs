#![allow(dead_code)]

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::actions::{ActionId, SafetyClass};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExperimentId(pub String);

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CandidateAction {
    pub action_id: ActionId,
    pub action_kind: String,
    pub safety_class: SafetyClass,
    pub description: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum AutotuneDecision {
    Noop {
        reason: String,
    },
    Suggest {
        candidate: CandidateAction,
        reason: String,
    },
    StartExperiment {
        candidate: CandidateAction,
        reason: String,
    },
    KeepCurrent {
        experiment_id: ExperimentId,
        reason: String,
    },
    Revert {
        experiment_id: ExperimentId,
        reason: String,
    },
    EnterCooldown {
        duration: Duration,
        reason: String,
    },
    Fault {
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate() -> CandidateAction {
        CandidateAction {
            action_id: ActionId("cpu-affinity-profile:test".to_owned()),
            action_kind: "cpu_affinity_profile".to_owned(),
            safety_class: SafetyClass::ReversibleLowRisk,
            description: "apply CPU affinity profile 'test' to process tree 1234".to_owned(),
        }
    }

    #[test]
    fn decision_serializes_and_roundtrips_candidate() {
        let decision = AutotuneDecision::StartExperiment {
            candidate: candidate(),
            reason: "candidate passed safety gate".to_owned(),
        };

        let json = serde_json::to_string(&decision).unwrap();
        let roundtrip: AutotuneDecision = serde_json::from_str(&json).unwrap();

        assert_eq!(roundtrip, decision);
    }

    #[test]
    fn decision_serializes_and_roundtrips_cooldown() {
        let decision = AutotuneDecision::EnterCooldown {
            duration: Duration::from_secs(60),
            reason: "experiment completed".to_owned(),
        };

        let json = serde_json::to_string(&decision).unwrap();
        let roundtrip: AutotuneDecision = serde_json::from_str(&json).unwrap();

        assert_eq!(roundtrip, decision);
    }
}
