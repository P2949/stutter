#![allow(dead_code)]

use crate::profiles::Profile;

#[derive(Clone, Debug)]
pub enum CandidateAction {
    CpuAffinityProfile {
        profile_name: String,
        profile: Profile,
        tree_pid: u32,
    },
    #[cfg(test)]
    Fake {
        action_id: crate::actions::ActionId,
        safety_class: crate::actions::SafetyClass,
    },
}

impl CandidateAction {
    pub fn cpu_affinity_profile(profile: Profile, tree_pid: u32) -> Self {
        Self::CpuAffinityProfile {
            profile_name: profile.name.clone(),
            profile,
            tree_pid,
        }
    }

    pub fn profile_name(&self) -> &str {
        match self {
            Self::CpuAffinityProfile { profile_name, .. } => profile_name,
            #[cfg(test)]
            Self::Fake { .. } => "fake-profile",
        }
    }

    pub fn tree_pid(&self) -> u32 {
        match self {
            Self::CpuAffinityProfile { tree_pid, .. } => *tree_pid,
            #[cfg(test)]
            Self::Fake { .. } => 0,
        }
    }

    pub fn action_kind(&self) -> &'static str {
        match self {
            Self::CpuAffinityProfile { .. } => "cpu_affinity_profile",
            #[cfg(test)]
            Self::Fake { .. } => "fake",
        }
    }

    pub fn safety_class(&self) -> crate::actions::SafetyClass {
        match self {
            Self::CpuAffinityProfile { .. } => crate::actions::SafetyClass::ReversibleLowRisk,
            #[cfg(test)]
            Self::Fake { safety_class, .. } => safety_class.clone(),
        }
    }

    pub fn action_id(&self) -> crate::actions::ActionId {
        match self {
            Self::CpuAffinityProfile { profile_name, .. } => {
                crate::actions::ActionId(format!("cpu-affinity-profile:{}", profile_name))
            }
            #[cfg(test)]
            Self::Fake { action_id, .. } => action_id.clone(),
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Self::CpuAffinityProfile {
                profile_name,
                tree_pid,
                ..
            } => {
                format!(
                    "apply CPU affinity profile '{}' to process tree {}",
                    profile_name, tree_pid
                )
            }
            #[cfg(test)]
            Self::Fake { action_id, .. } => format!("fake action {}", action_id.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{affinity::CpuMask, process_tree::TaskClass, profiles::ProfileRule};

    fn test_profile() -> Profile {
        Profile {
            name: "game-main".to_owned(),
            rules: vec![ProfileRule {
                affinity: CpuMask::parse("0").unwrap(),
                match_class: vec![TaskClass::Game],
                match_comm: Vec::new(),
            }],
        }
    }

    #[test]
    fn cpu_affinity_profile_candidate_copies_profile_name() {
        let profile = test_profile();
        let candidate = CandidateAction::cpu_affinity_profile(profile, 1234);

        match candidate {
            CandidateAction::CpuAffinityProfile {
                profile_name,
                profile,
                tree_pid,
            } => {
                assert_eq!(profile_name, "game-main");
                assert_eq!(profile.name, "game-main");
                assert_eq!(tree_pid, 1234);
            }
            #[cfg(test)]
            _ => unreachable!(),
        }
    }

    #[test]
    fn candidate_helpers_return_stable_metadata() {
        let candidate = CandidateAction::cpu_affinity_profile(test_profile(), 1234);

        assert_eq!(candidate.profile_name(), "game-main");
        assert_eq!(candidate.tree_pid(), 1234);
        assert_eq!(candidate.action_kind(), "cpu_affinity_profile");
    }
}
