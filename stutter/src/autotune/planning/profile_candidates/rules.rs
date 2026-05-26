use std::collections::BTreeSet;

use super::{
    super::candidate::CandidateAction, gaming::*, helpers::*, topology::CandidateCpuLayout,
    validate::*, *,
};
use crate::{profiles::Profile, topology::TopologyModel};

pub fn generate_topology_aware_profile_candidates(
    topology: &TopologyModel,
    tree_pid: u32,
) -> Vec<CandidateAction> {
    generate_topology_aware_profile_candidate_plan(
        topology,
        tree_pid,
        &GeneratedCpuSetPolicy::default(),
    )
    .optimization_candidates
}

pub fn generate_topology_aware_profile_candidates_with_policy(
    topology: &TopologyModel,
    tree_pid: u32,
    policy: &GeneratedCpuSetPolicy,
) -> GeneratedProfileCandidatePlan {
    generate_topology_aware_profile_candidate_plan(topology, tree_pid, policy)
}

pub fn generate_topology_aware_profile_candidate_plan(
    topology: &TopologyModel,
    tree_pid: u32,
    policy: &GeneratedCpuSetPolicy,
) -> GeneratedProfileCandidatePlan {
    let generated = generate_topology_aware_profile_plan(topology, policy);
    let mut optimization_candidates = Vec::new();
    let mut recovery_fallback = None;

    for profile in generated.profiles {
        let candidate = CandidateAction::cpu_affinity_profile(profile.clone(), tree_pid);
        if is_baseline_online_profile(&profile.name) {
            recovery_fallback = Some(candidate);
        } else {
            optimization_candidates.push(candidate);
        }
    }

    GeneratedProfileCandidatePlan {
        optimization_candidates,
        recovery_fallback,
        rejected: generated.rejected,
    }
}

pub fn generate_topology_aware_profiles(topology: &TopologyModel) -> Vec<Profile> {
    generate_topology_aware_profile_plan(topology, &GeneratedCpuSetPolicy::default()).profiles
}

pub fn generate_topology_aware_profiles_with_policy(
    topology: &TopologyModel,
    policy: &GeneratedCpuSetPolicy,
) -> GeneratedTopologyProfilePlan {
    generate_topology_aware_profile_plan(topology, policy)
}

pub fn generate_topology_aware_profile_plan(
    topology: &TopologyModel,
    policy: &GeneratedCpuSetPolicy,
) -> GeneratedTopologyProfilePlan {
    let Some(layout) = CandidateCpuLayout::from_topology(topology) else {
        return GeneratedTopologyProfilePlan {
            profiles: Vec::new(),
            rejected: vec![RejectedCandidateProfile {
                profile_name: "<topology>".to_owned(),
                reason: "no online CPUs available for topology-aware generation".to_owned(),
            }],
        };
    };

    let mut profiles = Vec::new();
    let mut rejected = Vec::new();

    for profile in [
        baseline_online_profile(&layout),
        game_isolate_render_profile(&layout),
        game_compositor_separate_profile(&layout),
        helper_spread_profile(&layout),
        wine_server_dedicated_profile(&layout),
        avoid_smt_contention_profile(&layout),
    ] {
        match validate_generated_profile(&profile, topology, policy) {
            Ok(()) => profiles.push(profile),
            Err(reason) => rejected.push(RejectedCandidateProfile {
                profile_name: profile.name,
                reason,
            }),
        }
    }

    GeneratedTopologyProfilePlan { profiles, rejected }
}
pub fn generate_profile_candidates(
    profiles: &[Profile],
    tree_pid: u32,
    current_profile: Option<&str>,
) -> Vec<CandidateAction> {
    generate_profile_candidate_plan(profiles, tree_pid, current_profile).optimization_candidates
}

pub fn generate_profile_candidates_for_observation(
    profiles: &[Profile],
    observation: &crate::autotune::observation::AutotuneObservation,
) -> Vec<CandidateAction> {
    generate_profile_candidate_plan_for_observation(profiles, observation).optimization_candidates
}

pub fn generate_profile_candidate_plan_for_observation(
    profiles: &[Profile],
    observation: &crate::autotune::observation::AutotuneObservation,
) -> GeneratedProfileCandidatePlan {
    let Some(tree_pid) = observation.target_root_pid else {
        return GeneratedProfileCandidatePlan {
            optimization_candidates: Vec::new(),
            recovery_fallback: None,
            rejected: Vec::new(),
        };
    };
    let recently_failed_profiles = BTreeSet::new();
    generate_profile_candidate_plan_with_checker(
        profiles,
        tree_pid,
        None,
        &recently_failed_profiles,
        |profile| {
            let matched_tasks = crate::profiles::profile_matched_task_count_from_snapshots(
                &observation.active_tasks,
                profile,
            );
            Ok(CandidateProfileStatus {
                matched_tasks,
                dry_run_tasks: matched_tasks,
            })
        },
    )
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

pub(crate) fn generate_profile_candidate_plan_with_checker<F>(
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

pub(crate) fn check_profile_for_candidate(
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
