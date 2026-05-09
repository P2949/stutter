#![cfg(feature = "autotune-controller")]

use std::time::Duration;

pub use super::{candidate::CandidateAction, experiment::ExperimentId};

#[derive(Clone, Debug)]
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
    use crate::{
        affinity::CpuMask,
        process_tree::TaskClass,
        profiles::{Profile, ProfileRule},
    };

    fn test_profile() -> Profile {
        Profile {
            name: "test".to_owned(),
            rules: vec![ProfileRule {
                affinity: Some(CpuMask::parse("0").unwrap()),
                nice: None,
                ionice: None,
                match_class: vec![TaskClass::Game],
                match_comm: Vec::new(),
            }],
        }
    }

    fn candidate() -> CandidateAction {
        CandidateAction::cpu_affinity_profile(test_profile(), 1234)
    }

    #[test]
    fn decision_can_hold_candidate() {
        let _decision = AutotuneDecision::StartExperiment {
            candidate: candidate(),
            reason: "candidate passed safety gate".to_owned(),
        };
    }

    #[test]
    fn decision_can_hold_cooldown() {
        let _decision = AutotuneDecision::EnterCooldown {
            duration: Duration::from_secs(60),
            reason: "experiment completed".to_owned(),
        };
    }
}
