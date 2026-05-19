//! CPU-affinity profile candidate generation.

use std::collections::BTreeSet;

use super::candidate::CandidateAction;
use crate::{
    process_tree::{CompiledPattern, TaskClass},
    profiles::{Profile, ProfileRule},
    topology::{CoreInfo, TopologyModel, cpu_mask_to_vec, cpus_to_mask, sorted_unique},
};

#[derive(Clone, Debug)]
pub struct GeneratedProfileCandidatePlan {
    pub optimization_candidates: Vec<CandidateAction>,
    pub recovery_fallback: Option<CandidateAction>,
    pub rejected: Vec<RejectedCandidateProfile>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeneratedCpuSetPolicy {
    pub allowed_cpus: Option<crate::affinity::CpuMask>,
    pub denied_cpus: Option<crate::affinity::CpuMask>,
    pub min_render_cpus: usize,
    pub min_game_cpus: usize,
    pub min_compositor_cpus: usize,
    pub min_background_cpus: usize,
}

impl Default for GeneratedCpuSetPolicy {
    fn default() -> Self {
        Self {
            allowed_cpus: None,
            denied_cpus: None,
            min_render_cpus: 1,
            min_game_cpus: 1,
            min_compositor_cpus: 1,
            min_background_cpus: 2,
        }
    }
}

#[derive(Clone, Debug)]
pub struct GeneratedTopologyProfilePlan {
    pub profiles: Vec<Profile>,
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

#[derive(Clone, Debug)]
struct CandidateCpuLayout {
    online_mask: crate::affinity::CpuMask,
    render_mask: crate::affinity::CpuMask,
    worker_mask: crate::affinity::CpuMask,
    compositor_mask: crate::affinity::CpuMask,
    helper_mask: crate::affinity::CpuMask,
    wine_server_mask: crate::affinity::CpuMask,
    separate_game_mask: crate::affinity::CpuMask,
    separate_compositor_mask: crate::affinity::CpuMask,
    avoid_smt_render_mask: crate::affinity::CpuMask,
    avoid_smt_compositor_mask: crate::affinity::CpuMask,
    avoid_smt_worker_mask: crate::affinity::CpuMask,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CoreChoice {
    package_id: Option<u32>,
    core_id: Option<u32>,
    numa_node: Option<u32>,
    cpus: Vec<u32>,
    primary_cpu: u32,
    max_mhz: Option<u64>,
}

impl CandidateCpuLayout {
    fn from_topology(topology: &TopologyModel) -> Option<Self> {
        let online_cpus = topology_online_cpus(topology);
        if online_cpus.is_empty() {
            return None;
        }

        let online_mask = cpus_to_mask(&online_cpus)?;
        let cores = topology_core_choices(topology, &online_cpus);
        let render_core_count = if cores.len() >= 6 { 2 } else { 1 };

        let render_cores = cores
            .iter()
            .take(render_core_count)
            .cloned()
            .collect::<Vec<_>>();
        let render_primary_cpus = render_cores
            .iter()
            .map(|core| core.primary_cpu)
            .collect::<Vec<_>>();
        let render_all_cpus = flatten_core_cpus(&render_cores);

        let non_render_cores = cores
            .iter()
            .filter(|core| !render_cores.iter().any(|render| same_core(render, core)))
            .cloned()
            .collect::<Vec<_>>();

        let worker_cpus = if non_render_cores.is_empty() {
            online_cpus.clone()
        } else {
            flatten_core_cpus(&non_render_cores)
        };

        let compositor_core = non_render_cores
            .iter()
            .find(|core| core.cpus.iter().all(|cpu| !render_all_cpus.contains(cpu)))
            .cloned()
            .or_else(|| non_render_cores.first().cloned())
            .or_else(|| cores.first().cloned())?;

        let compositor_cpus = vec![compositor_core.primary_cpu];

        let wine_core = non_render_cores
            .iter()
            .find(|core| !same_core(core, &compositor_core))
            .cloned()
            .or_else(|| Some(compositor_core.clone()))?;
        let wine_server_cpus = wine_core.cpus.clone();

        let separate_compositor_core = compositor_core.clone();
        let separate_compositor_cpus = separate_compositor_core.cpus.clone();
        let separate_game_cpus = cores
            .iter()
            .filter(|core| !same_core(core, &separate_compositor_core))
            .flat_map(|core| core.cpus.iter().copied())
            .collect::<Vec<_>>();
        let separate_game_cpus = if separate_game_cpus.is_empty() {
            online_cpus.clone()
        } else {
            separate_game_cpus
        };

        let render_sibling_set = topology
            .smt_siblings
            .get(render_primary_cpus.first().unwrap_or(&online_cpus[0]))
            .cloned()
            .unwrap_or_else(|| render_all_cpus.clone());

        let avoid_smt_compositor_core = non_render_cores
            .iter()
            .find(|core| {
                core.cpus
                    .iter()
                    .all(|cpu| !render_sibling_set.contains(cpu))
            })
            .cloned()
            .or_else(|| non_render_cores.first().cloned())
            .or_else(|| cores.first().cloned())?;

        let avoid_smt_worker_cpus = online_cpus
            .iter()
            .copied()
            .filter(|cpu| {
                !render_sibling_set.contains(cpu) && !avoid_smt_compositor_core.cpus.contains(cpu)
            })
            .collect::<Vec<_>>();
        let avoid_smt_worker_cpus = if avoid_smt_worker_cpus.is_empty() {
            worker_cpus.clone()
        } else {
            avoid_smt_worker_cpus
        };

        Some(Self {
            online_mask,
            render_mask: cpus_to_mask(&render_primary_cpus)?,
            worker_mask: cpus_to_mask(&worker_cpus)?,
            compositor_mask: cpus_to_mask(&compositor_cpus)?,
            helper_mask: cpus_to_mask(&worker_cpus)?,
            wine_server_mask: cpus_to_mask(&wine_server_cpus)?,
            separate_game_mask: cpus_to_mask(&separate_game_cpus)?,
            separate_compositor_mask: cpus_to_mask(&separate_compositor_cpus)?,
            avoid_smt_render_mask: cpus_to_mask(&render_primary_cpus)?,
            avoid_smt_compositor_mask: cpus_to_mask(&[avoid_smt_compositor_core.primary_cpu])?,
            avoid_smt_worker_mask: cpus_to_mask(&avoid_smt_worker_cpus)?,
        })
    }
}

fn topology_online_cpus(topology: &TopologyModel) -> Vec<u32> {
    let online = topology.online_cpu_ids();
    if online.is_empty() {
        cpu_mask_to_vec(&topology.online_cpus)
    } else {
        online
    }
}

fn topology_core_choices(topology: &TopologyModel, online_cpus: &[u32]) -> Vec<CoreChoice> {
    let mut choices = topology
        .cores
        .iter()
        .filter(|core| core.is_online)
        .filter_map(|core| core_choice_from_core(core, online_cpus))
        .collect::<Vec<_>>();

    if choices.is_empty() {
        choices = online_cpus
            .iter()
            .copied()
            .map(|cpu| CoreChoice {
                package_id: None,
                core_id: Some(cpu),
                numa_node: None,
                cpus: vec![cpu],
                primary_cpu: cpu,
                max_mhz: topology.cpu_info(cpu).and_then(|info| info.max_mhz),
            })
            .collect();
    }

    choices.sort_by(|left, right| {
        right
            .max_mhz
            .unwrap_or(0)
            .cmp(&left.max_mhz.unwrap_or(0))
            .then_with(|| left.package_id.cmp(&right.package_id))
            .then_with(|| left.numa_node.cmp(&right.numa_node))
            .then_with(|| left.core_id.cmp(&right.core_id))
            .then_with(|| left.primary_cpu.cmp(&right.primary_cpu))
    });

    choices
}

fn core_choice_from_core(core: &CoreInfo, online_cpus: &[u32]) -> Option<CoreChoice> {
    let cpus = sorted_unique(
        core.cpus
            .iter()
            .copied()
            .filter(|cpu| online_cpus.contains(cpu))
            .collect(),
    );
    let primary_cpu = cpus.first().copied()?;

    Some(CoreChoice {
        package_id: core.package_id,
        core_id: core.core_id,
        numa_node: core.numa_node,
        cpus,
        primary_cpu,
        max_mhz: core.max_mhz,
    })
}

fn same_core(left: &CoreChoice, right: &CoreChoice) -> bool {
    left.package_id == right.package_id
        && left.core_id == right.core_id
        && left.numa_node == right.numa_node
}

fn flatten_core_cpus(cores: &[CoreChoice]) -> Vec<u32> {
    sorted_unique(
        cores
            .iter()
            .flat_map(|core| core.cpus.iter().copied())
            .collect(),
    )
}

fn baseline_online_profile(layout: &CandidateCpuLayout) -> Profile {
    Profile {
        name: "baseline-online".to_owned(),
        rules: vec![ProfileRule {
            affinity: Some(layout.online_mask.clone()),
            nice: None,
            ionice: None,
            match_class: Vec::new(),
            match_comm: Vec::new(),
        }],
    }
}

fn game_isolate_render_profile(layout: &CandidateCpuLayout) -> Profile {
    Profile {
        name: "game-isolate-render".to_owned(),
        rules: vec![
            profile_rule(
                &layout.render_mask,
                vec![TaskClass::GameRenderThread],
                Vec::new(),
            ),
            profile_rule(
                &layout.render_mask,
                vec![TaskClass::Game],
                vec!["RenderThread", "Main"],
            ),
            profile_rule(
                &layout.compositor_mask,
                vec![TaskClass::Compositor, TaskClass::GameScope],
                Vec::new(),
            ),
            profile_rule(
                &layout.wine_server_mask,
                vec![TaskClass::WineServer],
                Vec::new(),
            ),
            profile_rule(
                &layout.worker_mask,
                vec![TaskClass::GameWorkerThread, TaskClass::GameHelper],
                Vec::new(),
            ),
            profile_rule(&layout.worker_mask, vec![TaskClass::Game], Vec::new()),
        ],
    }
}

fn game_compositor_separate_profile(layout: &CandidateCpuLayout) -> Profile {
    Profile {
        name: "game-compositor-separate".to_owned(),
        rules: vec![
            profile_rule(
                &layout.separate_compositor_mask,
                vec![TaskClass::Compositor, TaskClass::GameScope],
                Vec::new(),
            ),
            profile_rule(
                &layout.separate_game_mask,
                vec![
                    TaskClass::Game,
                    TaskClass::GameRenderThread,
                    TaskClass::GameWorkerThread,
                    TaskClass::GameHelper,
                    TaskClass::WineServer,
                ],
                Vec::new(),
            ),
        ],
    }
}

fn helper_spread_profile(layout: &CandidateCpuLayout) -> Profile {
    Profile {
        name: "helper-spread".to_owned(),
        rules: vec![profile_rule(
            &layout.helper_mask,
            vec![
                TaskClass::GameHelper,
                TaskClass::GameWorkerThread,
                TaskClass::SteamRuntime,
                TaskClass::Helper,
            ],
            Vec::new(),
        )],
    }
}

fn wine_server_dedicated_profile(layout: &CandidateCpuLayout) -> Profile {
    Profile {
        name: "wine-server-dedicated".to_owned(),
        rules: vec![profile_rule(
            &layout.wine_server_mask,
            vec![TaskClass::WineServer],
            Vec::new(),
        )],
    }
}

fn avoid_smt_contention_profile(layout: &CandidateCpuLayout) -> Profile {
    Profile {
        name: "avoid-smt-contention".to_owned(),
        rules: vec![
            profile_rule(
                &layout.avoid_smt_render_mask,
                vec![TaskClass::GameRenderThread],
                Vec::new(),
            ),
            profile_rule(
                &layout.avoid_smt_render_mask,
                vec![TaskClass::Game],
                vec!["RenderThread", "Main"],
            ),
            profile_rule(
                &layout.avoid_smt_compositor_mask,
                vec![TaskClass::Compositor, TaskClass::GameScope],
                Vec::new(),
            ),
            profile_rule(
                &layout.avoid_smt_worker_mask,
                vec![
                    TaskClass::GameWorkerThread,
                    TaskClass::GameHelper,
                    TaskClass::WineServer,
                ],
                Vec::new(),
            ),
        ],
    }
}

fn profile_rule(
    affinity: &crate::affinity::CpuMask,
    match_class: Vec<TaskClass>,
    match_comm: Vec<&str>,
) -> ProfileRule {
    ProfileRule {
        affinity: Some(affinity.clone()),
        nice: None,
        ionice: None,
        match_class,
        match_comm: match_comm
            .into_iter()
            .map(|pattern| {
                CompiledPattern::new(pattern.to_owned())
                    // invariant: generated profile patterns are static literals validated by tests.
                    .expect("generated candidate command pattern must be valid")
            })
            .collect(),
    }
}

pub(crate) fn validate_generated_profile(
    profile: &Profile,
    topology: &TopologyModel,
    policy: &GeneratedCpuSetPolicy,
) -> Result<(), String> {
    if profile.rules.is_empty() {
        return Err("generated profile has no rules".to_owned());
    }

    let online = &topology.online_cpus;

    for (index, rule) in profile.rules.iter().enumerate() {
        validate_generated_rule_mask(profile, index, rule, online, policy)?;
    }

    validate_render_and_game_minimums(profile, policy)?;
    validate_compositor_minimum(profile, policy)?;
    validate_background_capacity(profile, policy)?;

    Ok(())
}

fn validate_generated_rule_mask(
    profile: &Profile,
    index: usize,
    rule: &ProfileRule,
    online: &crate::affinity::CpuMask,
    policy: &GeneratedCpuSetPolicy,
) -> Result<(), String> {
    let Some(affinity) = rule.affinity.as_ref() else {
        return Err(format!("rule {index} is missing CPU mask"));
    };

    if affinity.is_empty() {
        return Err(format!("rule {index} has empty CPU mask"));
    }

    if !affinity.is_subset_of(online) {
        return Err(format!(
            "rule {index} requests offline CPUs: requested={} online={}",
            affinity.to_range_string(),
            online.to_range_string()
        ));
    }

    if let Some(allowed) = &policy.allowed_cpus
        && !affinity.is_subset_of(allowed)
    {
        return Err(format!(
            "rule {index} violates allowed CPU set: requested={} allowed={}",
            affinity.to_range_string(),
            allowed.to_range_string()
        ));
    }

    if let Some(denied) = &policy.denied_cpus {
        let requested = cpu_mask_to_vec(affinity);
        let denied = cpu_mask_to_vec(denied);
        let overlap = requested
            .into_iter()
            .filter(|cpu| denied.contains(cpu))
            .collect::<Vec<_>>();

        if !overlap.is_empty() {
            return Err(format!(
                "rule {index} violates denied CPU set: requested={} denied={} overlap={}",
                affinity.to_range_string(),
                policy
                    .denied_cpus
                    .as_ref()
                    .map(|mask| mask.to_range_string())
                    .unwrap_or_default(),
                crate::topology::cpus_to_range_string(&overlap)
            ));
        }
    }

    if profile.name != "baseline-online" && rule_matches_audio_or_input(rule) {
        return Err(format!(
            "rule {index} targets critical realtime/input classes in generated profile"
        ));
    }

    Ok(())
}

fn validate_render_and_game_minimums(
    profile: &Profile,
    policy: &GeneratedCpuSetPolicy,
) -> Result<(), String> {
    for (index, rule) in profile.rules.iter().enumerate() {
        let Some(affinity) = rule.affinity.as_ref() else {
            return Err(format!("rule {index} is missing CPU mask"));
        };
        let cpu_count = cpu_mask_to_vec(affinity).len();

        if rule_matches_render_or_main_game(rule) && cpu_count < policy.min_render_cpus {
            return Err(format!(
                "rule {index} gives render/main game work fewer than minimum CPUs: cpus={} min={}",
                cpu_count, policy.min_render_cpus
            ));
        }

        if rule_matches_game_work(rule) && cpu_count < policy.min_game_cpus {
            return Err(format!(
                "rule {index} gives game work fewer than minimum CPUs: cpus={} min={}",
                cpu_count, policy.min_game_cpus
            ));
        }
    }

    Ok(())
}

fn validate_compositor_minimum(
    profile: &Profile,
    policy: &GeneratedCpuSetPolicy,
) -> Result<(), String> {
    for (index, rule) in profile.rules.iter().enumerate() {
        if !rule_matches_compositor_or_gamescope(rule) {
            continue;
        }

        let Some(affinity) = rule.affinity.as_ref() else {
            return Err(format!("rule {index} is missing CPU mask"));
        };
        let cpu_count = cpu_mask_to_vec(affinity).len();
        if cpu_count < policy.min_compositor_cpus {
            return Err(format!(
                "rule {index} gives compositor/gamescope fewer than minimum CPUs: cpus={} min={}",
                cpu_count, policy.min_compositor_cpus
            ));
        }
    }

    Ok(())
}

fn validate_background_capacity(
    profile: &Profile,
    policy: &GeneratedCpuSetPolicy,
) -> Result<(), String> {
    if profile.name == "baseline-online" {
        return Ok(());
    }

    for (index, rule) in profile.rules.iter().enumerate() {
        if !rule_matches_background_or_helper_work(rule) {
            continue;
        }

        let Some(affinity) = rule.affinity.as_ref() else {
            return Err(format!("rule {index} is missing CPU mask"));
        };
        let cpu_count = cpu_mask_to_vec(affinity).len();
        if cpu_count < policy.min_background_cpus {
            return Err(format!(
                "rule {index} pushes background/helper work onto too few CPUs: cpus={} min={}",
                cpu_count, policy.min_background_cpus
            ));
        }
    }

    Ok(())
}

fn rule_matches_render_or_main_game(rule: &ProfileRule) -> bool {
    rule.match_class.contains(&TaskClass::GameRenderThread)
        || (rule.match_class.contains(&TaskClass::Game)
            && rule.match_comm.iter().any(|pattern| {
                let raw = pattern.raw().to_ascii_lowercase();
                raw.contains("render") || raw.contains("main")
            }))
}

fn rule_matches_game_work(rule: &ProfileRule) -> bool {
    rule.match_class.iter().any(|class| {
        matches!(
            class,
            TaskClass::Game
                | TaskClass::GameRenderThread
                | TaskClass::GameWorkerThread
                | TaskClass::GameHelper
                | TaskClass::WineServer
        )
    })
}

fn rule_matches_compositor_or_gamescope(rule: &ProfileRule) -> bool {
    rule.match_class
        .iter()
        .any(|class| matches!(class, TaskClass::Compositor | TaskClass::GameScope))
}

fn rule_matches_background_or_helper_work(rule: &ProfileRule) -> bool {
    rule.match_class.iter().any(|class| {
        matches!(
            class,
            TaskClass::GameWorkerThread
                | TaskClass::GameHelper
                | TaskClass::SteamRuntime
                | TaskClass::Helper
                | TaskClass::WineServer
        )
    })
}

fn rule_matches_audio_or_input(rule: &ProfileRule) -> bool {
    rule.match_class
        .iter()
        .any(|class| matches!(class, TaskClass::AudioRealtime | TaskClass::Input))
}

fn is_baseline_online_profile(name: &str) -> bool {
    name == "baseline-online"
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
