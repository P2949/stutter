#![allow(dead_code)]

use std::collections::BTreeSet;

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

#[derive(Clone, Debug)]
pub struct GeneratedProfileCandidatePlan {
    pub optimization_candidates: Vec<CandidateAction>,
    pub recovery_fallback: Option<CandidateAction>,
    pub rejected: Vec<RejectedCandidateProfile>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RejectedCandidateProfile {
    pub profile_name: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandidateProfileStatus {
    pub matched_tasks: usize,
    pub dry_run_tasks: usize,
}

pub fn generate_profile_candidates(
    profiles: &[Profile],
    tree_pid: u32,
    current_profile: Option<&str>,
) -> Vec<CandidateAction> {
    generate_profile_candidate_plan(profiles, tree_pid, current_profile).optimization_candidates
}

pub fn generate_profile_candidate_plan(
    profiles: &[Profile],
    tree_pid: u32,
    current_profile: Option<&str>,
) -> GeneratedProfileCandidatePlan {
    let recently_failed_profiles = BTreeSet::new();
    generate_profile_candidate_plan_with_history(
        profiles,
        tree_pid,
        current_profile,
        &recently_failed_profiles,
    )
}

pub fn generate_profile_candidate_plan_with_history(
    profiles: &[Profile],
    tree_pid: u32,
    current_profile: Option<&str>,
    recently_failed_profiles: &BTreeSet<String>,
) -> GeneratedProfileCandidatePlan {
    generate_profile_candidate_plan_with_checker(
        profiles,
        tree_pid,
        current_profile,
        recently_failed_profiles,
        |profile| check_profile_for_candidate(tree_pid, profile),
    )
}

fn generate_profile_candidate_plan_with_checker<F>(
    profiles: &[Profile],
    tree_pid: u32,
    current_profile: Option<&str>,
    recently_failed_profiles: &BTreeSet<String>,
    mut check_profile: F,
) -> GeneratedProfileCandidatePlan
where
    F: FnMut(&Profile) -> anyhow::Result<CandidateProfileStatus>,
{
    let mut preferred_candidates = Vec::new();
    let mut recently_failed_candidates = Vec::new();
    let mut recovery_fallback = None;
    let mut rejected = Vec::new();

    for profile in profiles {
        if Some(profile.name.as_str()) == current_profile {
            rejected.push(RejectedCandidateProfile {
                profile_name: profile.name.clone(),
                reason: "current profile".to_owned(),
            });
            continue;
        }

        let status = match check_profile(profile) {
            Ok(status) => status,
            Err(err) => {
                rejected.push(RejectedCandidateProfile {
                    profile_name: profile.name.clone(),
                    reason: format!("dry-run failed: {err:#}"),
                });
                continue;
            }
        };

        if status.matched_tasks == 0 {
            rejected.push(RejectedCandidateProfile {
                profile_name: profile.name.clone(),
                reason: "zero matched tasks".to_owned(),
            });
            continue;
        }

        let candidate = CandidateAction::cpu_affinity_profile(profile.clone(), tree_pid);

        if is_baseline_online_profile(&profile.name) {
            recovery_fallback = Some(candidate);
            continue;
        }

        if recently_failed_profiles.contains(&profile.name) {
            recently_failed_candidates.push(candidate);
        } else {
            preferred_candidates.push(candidate);
        }
    }

    preferred_candidates.extend(recently_failed_candidates);

    GeneratedProfileCandidatePlan {
        optimization_candidates: preferred_candidates,
        recovery_fallback,
        rejected,
    }
}

fn check_profile_for_candidate(
    tree_pid: u32,
    profile: &Profile,
) -> anyhow::Result<CandidateProfileStatus> {
    let matched_tasks = crate::profiles::profile_matched_task_count_for_tree(tree_pid, profile);
    let dry_run_records = crate::profiles::apply_profile_to_tree(tree_pid, profile, false, true)?;

    Ok(CandidateProfileStatus {
        matched_tasks,
        dry_run_tasks: dry_run_records.len(),
    })
}

fn is_baseline_online_profile(profile_name: &str) -> bool {
    profile_name == "baseline-online"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{affinity::CpuMask, process_tree::TaskClass, profiles::ProfileRule};

    fn profile(name: &str) -> Profile {
        Profile {
            name: name.to_owned(),
            rules: vec![ProfileRule {
                affinity: CpuMask::parse("0").unwrap(),
                match_class: vec![TaskClass::Game],
                match_comm: Vec::new(),
            }],
        }
    }

    fn status_for_profile(profile: &Profile) -> anyhow::Result<CandidateProfileStatus> {
        match profile.name.as_str() {
            "dry-run-fails" => anyhow::bail!("intentional dry-run failure"),
            "zero-match" => Ok(CandidateProfileStatus {
                matched_tasks: 0,
                dry_run_tasks: 0,
            }),
            _ => Ok(CandidateProfileStatus {
                matched_tasks: 2,
                dry_run_tasks: 1,
            }),
        }
    }

    #[test]
    fn generate_profile_candidates_excludes_current_profile() {
        let profiles = vec![profile("current"), profile("candidate")];

        let plan = generate_profile_candidate_plan_with_checker(
            &profiles,
            1234,
            Some("current"),
            &BTreeSet::new(),
            status_for_profile,
        );

        assert_eq!(plan.optimization_candidates.len(), 1);
        assert_eq!(plan.optimization_candidates[0].profile_name(), "candidate");
        assert!(
            plan.rejected
                .iter()
                .any(|rejected| rejected.profile_name == "current"
                    && rejected.reason == "current profile")
        );
    }

    #[test]
    fn generate_profile_candidates_excludes_profiles_that_fail_dry_run() {
        let profiles = vec![profile("dry-run-fails"), profile("candidate")];

        let plan = generate_profile_candidate_plan_with_checker(
            &profiles,
            1234,
            None,
            &BTreeSet::new(),
            status_for_profile,
        );

        assert_eq!(plan.optimization_candidates.len(), 1);
        assert_eq!(plan.optimization_candidates[0].profile_name(), "candidate");
        assert!(
            plan.rejected
                .iter()
                .any(|rejected| rejected.profile_name == "dry-run-fails"
                    && rejected.reason.contains("dry-run failed"))
        );
    }

    #[test]
    fn generate_profile_candidates_excludes_zero_matched_tasks() {
        let profiles = vec![profile("zero-match"), profile("candidate")];

        let plan = generate_profile_candidate_plan_with_checker(
            &profiles,
            1234,
            None,
            &BTreeSet::new(),
            status_for_profile,
        );

        assert_eq!(plan.optimization_candidates.len(), 1);
        assert_eq!(plan.optimization_candidates[0].profile_name(), "candidate");
        assert!(
            plan.rejected
                .iter()
                .any(|rejected| rejected.profile_name == "zero-match"
                    && rejected.reason == "zero matched tasks")
        );
    }

    #[test]
    fn generate_profile_candidates_puts_recently_failed_names_last() {
        let profiles = vec![
            profile("recently-failed"),
            profile("fresh"),
            profile("another-fresh"),
        ];
        let recently_failed_profiles = BTreeSet::from(["recently-failed".to_owned()]);

        let plan = generate_profile_candidate_plan_with_checker(
            &profiles,
            1234,
            None,
            &recently_failed_profiles,
            status_for_profile,
        );

        let names = plan
            .optimization_candidates
            .iter()
            .map(CandidateAction::profile_name)
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["fresh", "another-fresh", "recently-failed"]);
    }

    #[test]
    fn baseline_online_is_recovery_fallback_not_optimization_candidate() {
        let profiles = vec![profile("baseline-online"), profile("candidate")];

        let plan = generate_profile_candidate_plan_with_checker(
            &profiles,
            1234,
            None,
            &BTreeSet::new(),
            status_for_profile,
        );

        assert_eq!(plan.optimization_candidates.len(), 1);
        assert_eq!(plan.optimization_candidates[0].profile_name(), "candidate");
        assert_eq!(
            plan.recovery_fallback
                .as_ref()
                .map(CandidateAction::profile_name),
            Some("baseline-online")
        );
    }

    #[test]
    fn public_generate_profile_candidates_returns_optimization_candidates_only() {
        let profiles = vec![profile("baseline-online"), profile("candidate")];

        let plan = generate_profile_candidate_plan_with_checker(
            &profiles,
            1234,
            None,
            &BTreeSet::new(),
            status_for_profile,
        );

        assert_eq!(plan.optimization_candidates.len(), 1);
        assert_eq!(plan.optimization_candidates[0].profile_name(), "candidate");
        assert!(plan.recovery_fallback.is_some());
    }

    #[test]
    fn candidate_helpers_return_stable_metadata() {
        let candidate = CandidateAction::cpu_affinity_profile(profile("game-main"), 1234);

        assert_eq!(candidate.profile_name(), "game-main");
        assert_eq!(candidate.tree_pid(), 1234);
        assert_eq!(candidate.action_kind(), "cpu_affinity_profile");
    }
}
