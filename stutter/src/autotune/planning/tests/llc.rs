use super::{
    super::profile_candidates::{GeneratedCpuSetPolicy, llc::*},
    support::*,
};
use crate::{
    process_tree::TaskClass,
    topology_v2::{LlcDomain, TopologyV2},
};

fn v2(domains: &[(u32, &[u32])]) -> TopologyV2 {
    TopologyV2 {
        llc_domains: domains
            .iter()
            .map(|(level, cpus)| LlcDomain {
                level: *level,
                cpus: cpus.to_vec(),
                size_kib: Some(32_768),
            })
            .collect(),
        capacity_classes: Vec::new(),
        larger_cache_cpus: Vec::new(),
        die_groups: Default::default(),
        frequency_domains: Vec::new(),
        numa_distance: Default::default(),
        gpu_locality: Vec::new(),
        effective_cpus: (0..8).collect(),
        fingerprint: String::new(),
    }
}

#[test]
fn two_cache_domains_pack_the_game_into_the_render_domain_and_move_the_compositor_out() {
    let topology = fake_topology_4c8t();
    let profile = llc_pack_profile(
        &topology,
        &v2(&[(3, &[0, 1, 2, 3]), (3, &[4, 5, 6, 7])]),
        &GeneratedCpuSetPolicy::default(),
    )
    .unwrap();
    assert_eq!(profile.name, LLC_PACK_PROFILE);
    let render = first_rule_for_class(&profile, TaskClass::GameRenderThread);
    let compositor = first_rule_for_class(&profile, TaskClass::Compositor);
    assert_eq!(affinity(render).to_range_string(), "0-3");
    assert_eq!(affinity(compositor).to_range_string(), "4-7");
}

#[test]
fn it_does_not_apply_without_two_verified_domains_and_the_reason_is_named() {
    let topology = fake_topology_4c8t();
    let policy = GeneratedCpuSetPolicy::default();
    for (case, topo) in [
        ("one domain", v2(&[(3, &[0, 1, 2, 3, 4, 5, 6, 7])])),
        ("no domains", v2(&[])),
        (
            "offline CPUs only",
            v2(&[(3, &[0, 1, 2, 3]), (3, &[40, 41])]),
        ),
    ] {
        let rejected = llc_pack_profile(&topology, &topo, &policy).unwrap_err();
        assert_eq!(rejected.profile_name, LLC_PACK_PROFILE, "{case}");
        assert!(!rejected.reason.is_empty(), "{case}");
    }
}

#[test]
fn a_policy_that_forbids_the_domain_rejects_the_hypothesis_like_any_generated_profile() {
    let topology = fake_topology_4c8t();
    let policy = GeneratedCpuSetPolicy {
        denied_cpus: Some(crate::affinity::CpuMask::parse("0-3").unwrap()),
        ..GeneratedCpuSetPolicy::default()
    };
    assert!(
        llc_pack_profile(
            &topology,
            &v2(&[(3, &[0, 1, 2, 3]), (3, &[4, 5, 6, 7])]),
            &policy
        )
        .is_err()
    );
}

#[test]
fn the_default_plan_is_untouched_and_the_hypothesis_is_appended_or_rejected_by_name() {
    let topology = fake_topology_4c8t();
    let policy = GeneratedCpuSetPolicy::default();
    let default =
        super::super::profile_candidates::generate_topology_aware_profile_plan(&topology, &policy);
    let two = generate_plan_with_cache_locality(
        &topology,
        &v2(&[(3, &[0, 1, 2, 3]), (3, &[4, 5, 6, 7])]),
        &policy,
    );
    assert_eq!(two.profiles.len(), default.profiles.len() + 1);
    assert_eq!(two.profiles.last().unwrap().name, LLC_PACK_PROFILE);
    let names = |profiles: &[crate::profiles::Profile]| {
        profiles.iter().map(|p| p.name.clone()).collect::<Vec<_>>()
    };
    assert_eq!(
        names(&two.profiles[..default.profiles.len()]),
        names(&default.profiles)
    );

    let one = generate_plan_with_cache_locality(
        &topology,
        &v2(&[(3, &[0, 1, 2, 3, 4, 5, 6, 7])]),
        &policy,
    );
    assert_eq!(names(&one.profiles), names(&default.profiles));
    assert_eq!(one.rejected.last().unwrap().profile_name, LLC_PACK_PROFILE);
}
