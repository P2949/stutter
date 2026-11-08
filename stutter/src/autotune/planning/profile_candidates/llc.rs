//! A cache-locality placement hypothesis from topology v2 (package W8).
//!
//! When the machine has more than one last-level-cache domain, one hypothesis is to keep the game
//! inside the domain that holds its render CPU so its threads share a cache, and to move the
//! compositor to the other domain. It is a *hypothesis*, never a conclusion: it goes through the
//! same validation as every generated profile, it is only produced when the domains are verified
//! from sysfs, and it grants no maturity (a new profile name is not in the autonomous registry).
use super::{
    GeneratedCpuSetPolicy, RejectedCandidateProfile, gaming::profile_rule,
    topology::CandidateCpuLayout, validate::validate_generated_profile,
};
use crate::{
    process_tree::TaskClass,
    profiles::Profile,
    topology::{TopologyModel, cpu_mask_to_vec, cpus_to_mask},
    topology_v2::TopologyV2,
};

pub const LLC_PACK_PROFILE: &str = "game-llc-pack";

/// The `game-llc-pack` profile, or why it does not apply on this machine.
pub fn llc_pack_profile(
    topology: &TopologyModel,
    v2: &TopologyV2,
    policy: &GeneratedCpuSetPolicy,
) -> Result<Profile, RejectedCandidateProfile> {
    let reject = |reason: &str| RejectedCandidateProfile {
        profile_name: LLC_PACK_PROFILE.to_owned(),
        reason: reason.to_owned(),
    };
    let layout = CandidateCpuLayout::from_topology(topology)
        .ok_or_else(|| reject("no online CPUs available"))?;
    let online = cpu_mask_to_vec(&topology.online_cpus);
    let last_level = v2
        .llc_domains
        .iter()
        .map(|domain| domain.level)
        .max()
        .ok_or_else(|| reject("no cache domain was read from sysfs"))?;
    let domains: Vec<Vec<u32>> = v2
        .llc_domains
        .iter()
        .filter(|domain| domain.level == last_level)
        .map(|domain| {
            domain
                .cpus
                .iter()
                .copied()
                .filter(|cpu| online.contains(cpu))
                .collect::<Vec<u32>>()
        })
        .filter(|cpus| !cpus.is_empty())
        .collect();
    if domains.len() < 2 {
        return Err(reject("only one last-level-cache domain: nothing to pack"));
    }
    let render_cpu = cpu_mask_to_vec(&layout.render_mask)
        .first()
        .copied()
        .ok_or_else(|| reject("no render CPU"))?;
    let home = domains
        .iter()
        .find(|cpus| cpus.contains(&render_cpu))
        .ok_or_else(|| reject("the render CPU is in no cache domain"))?;
    let others: Vec<u32> = domains
        .iter()
        .filter(|cpus| *cpus != home)
        .flatten()
        .copied()
        .collect();
    let home_mask = cpus_to_mask(home).ok_or_else(|| reject("cache domain exceeds the mask"))?;
    let other_mask =
        cpus_to_mask(&others).ok_or_else(|| reject("cache domain exceeds the mask"))?;
    let profile = Profile {
        name: LLC_PACK_PROFILE.to_owned(),
        rules: vec![
            profile_rule(
                &home_mask,
                vec![
                    TaskClass::Game,
                    TaskClass::GameRenderThread,
                    TaskClass::GameWorkerThread,
                    TaskClass::GameHelper,
                    TaskClass::WineServer,
                ],
                Vec::new(),
            ),
            profile_rule(
                &other_mask,
                vec![TaskClass::Compositor, TaskClass::GameScope],
                Vec::new(),
            ),
        ],
    };
    validate_generated_profile(&profile, topology, policy).map_err(|why| reject(&why))?;
    Ok(profile)
}

/// The default generated plan plus the cache-locality hypothesis when it applies. The default
/// plan is untouched; a machine without two verified cache domains gets exactly the default plan
/// and a named rejection for the hypothesis.
pub fn generate_plan_with_cache_locality(
    topology: &TopologyModel,
    v2: &TopologyV2,
    policy: &GeneratedCpuSetPolicy,
) -> super::GeneratedTopologyProfilePlan {
    let mut plan = super::generate_topology_aware_profile_plan(topology, policy);
    match llc_pack_profile(topology, v2, policy) {
        Ok(profile) => plan.profiles.push(profile),
        Err(rejected) => plan.rejected.push(rejected),
    }
    plan
}
