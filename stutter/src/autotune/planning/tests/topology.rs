use super::{
    super::{candidate::*, profile_candidates::*},
    support::*,
};
use crate::{
    process_tree::TaskClass,
    profiles::{Profile, ProfileRule},
};

#[test]
fn topology_aware_generation_includes_required_templates() {
    let topology = fake_topology_4c8t();
    let profiles = generate_topology_aware_profiles(&topology);
    let names = profiles
        .iter()
        .map(|profile| profile.name.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        names,
        vec![
            "baseline-online",
            "game-isolate-render",
            "game-compositor-separate",
            "helper-spread",
            "wine-server-dedicated",
            "avoid-smt-contention"
        ]
    );
}

#[test]
fn baseline_online_profile_matches_all_target_tasks_on_online_cpus() {
    let topology = fake_topology_4c8t();
    let profiles = generate_topology_aware_profiles(&topology);
    let baseline = profile_by_name(&profiles, "baseline-online");

    assert_eq!(baseline.rules.len(), 1);
    assert_eq!(affinity(&baseline.rules[0]).to_range_string(), "0-7");
    assert!(baseline.rules[0].match_class.is_empty());
    assert!(baseline.rules[0].match_comm.is_empty());
}

#[test]
fn game_isolate_render_uses_preferred_and_remaining_physical_cores() {
    let topology = fake_topology_4c8t();
    let profiles = generate_topology_aware_profiles(&topology);
    let profile = profile_by_name(&profiles, "game-isolate-render");

    let render_rule = first_rule_for_class(profile, TaskClass::GameRenderThread);
    let compositor_rule = first_rule_for_class(profile, TaskClass::Compositor);
    let worker_rule = first_rule_for_class(profile, TaskClass::GameWorkerThread);
    let wine_rule = first_rule_for_class(profile, TaskClass::WineServer);

    assert_eq!(affinity(render_rule).to_range_string(), "0");
    assert_eq!(affinity(compositor_rule).to_range_string(), "1");
    assert_eq!(affinity(worker_rule).to_range_string(), "1-3,5-7");
    assert_eq!(affinity(wine_rule).to_range_string(), "2,6");

    let game_main_rule = profile
        .rules
        .iter()
        .find(|rule| {
            rule.match_class.contains(&TaskClass::Game)
                && rule
                    .match_comm
                    .iter()
                    .any(|pattern| pattern.raw() == "Main")
        })
        .unwrap();
    assert_eq!(affinity(game_main_rule).to_range_string(), "0");
}

#[test]
fn game_compositor_separate_keeps_game_and_compositor_on_separate_physical_cores() {
    let topology = fake_topology_4c8t();
    let profiles = generate_topology_aware_profiles(&topology);
    let profile = profile_by_name(&profiles, "game-compositor-separate");

    let compositor_rule = first_rule_for_class(profile, TaskClass::Compositor);
    let game_rule = first_rule_for_class(profile, TaskClass::Game);

    assert_eq!(affinity(compositor_rule).to_range_string(), "1,5");
    assert_eq!(affinity(game_rule).to_range_string(), "0,2-4,6-7");
}

#[test]
fn helper_spread_uses_non_critical_cores() {
    let topology = fake_topology_4c8t();
    let profiles = generate_topology_aware_profiles(&topology);
    let profile = profile_by_name(&profiles, "helper-spread");
    let helper_rule = first_rule_for_class(profile, TaskClass::GameHelper);

    assert_eq!(affinity(helper_rule).to_range_string(), "1-3,5-7");
    assert!(
        helper_rule
            .match_class
            .contains(&TaskClass::GameWorkerThread)
    );
    assert!(helper_rule.match_class.contains(&TaskClass::SteamRuntime));
    assert!(helper_rule.match_class.contains(&TaskClass::Helper));
}

#[test]
fn wine_server_dedicated_uses_one_non_render_core_pair() {
    let topology = fake_topology_4c8t();
    let profiles = generate_topology_aware_profiles(&topology);
    let profile = profile_by_name(&profiles, "wine-server-dedicated");
    let wine_rule = first_rule_for_class(profile, TaskClass::WineServer);

    assert_eq!(affinity(wine_rule).to_range_string(), "2,6");
}

#[test]
fn avoid_smt_contention_keeps_render_and_compositor_off_smt_siblings() {
    let topology = fake_topology_4c8t();
    let profiles = generate_topology_aware_profiles(&topology);
    let profile = profile_by_name(&profiles, "avoid-smt-contention");

    let render_rule = first_rule_for_class(profile, TaskClass::GameRenderThread);
    let compositor_rule = first_rule_for_class(profile, TaskClass::Compositor);
    let worker_rule = first_rule_for_class(profile, TaskClass::GameWorkerThread);

    assert_eq!(affinity(render_rule).to_range_string(), "0");
    assert_eq!(affinity(compositor_rule).to_range_string(), "1");
    assert_eq!(affinity(worker_rule).to_range_string(), "2-3,6-7");

    let render_siblings = topology.smt_siblings.get(&0).unwrap();
    let compositor_cpus =
        crate::topology::parse_cpu_list(&affinity(compositor_rule).to_range_string()).unwrap();

    assert!(
        compositor_cpus
            .iter()
            .all(|cpu| !render_siblings.contains(cpu))
    );
}

#[test]
fn topology_aware_candidates_wrap_generated_profiles_for_tree_pid() {
    let topology = fake_topology_4c8t();
    let candidates = generate_topology_aware_profile_candidates(&topology, 1234);

    assert_eq!(candidates.len(), 5);
    assert_eq!(candidates[0].profile_name(), "game-isolate-render");
    assert_eq!(candidates[0].tree_pid(), 1234);
    assert_eq!(candidates[1].profile_name(), "game-compositor-separate");
    assert_eq!(candidates[1].action_kind(), "cpu_affinity_profile");
}

#[test]
fn generated_profile_plan_rejects_masks_outside_allowed_cpus() {
    let topology = fake_topology_4c8t();
    let policy = GeneratedCpuSetPolicy {
        allowed_cpus: Some(crate::affinity::CpuMask::parse("0-3").unwrap()),
        denied_cpus: None,
        min_render_cpus: 1,
        min_game_cpus: 1,
        min_compositor_cpus: 1,
        min_background_cpus: 2,
    };

    let plan = generate_topology_aware_profiles_with_policy(&topology, &policy);

    assert!(!plan.rejected.is_empty());
    assert!(plan.rejected.iter().any(|rejected| {
        rejected.reason.contains("violates allowed CPU set")
            && rejected.reason.contains("allowed=0-3")
    }));
}

#[test]
fn generated_profile_plan_rejects_masks_overlapping_denied_cpus() {
    let topology = fake_topology_4c8t();
    let policy = GeneratedCpuSetPolicy {
        allowed_cpus: None,
        denied_cpus: Some(crate::affinity::CpuMask::parse("1").unwrap()),
        min_render_cpus: 1,
        min_game_cpus: 1,
        min_compositor_cpus: 1,
        min_background_cpus: 2,
    };

    let plan = generate_topology_aware_profiles_with_policy(&topology, &policy);

    assert!(!plan.rejected.is_empty());
    assert!(plan.rejected.iter().any(|rejected| {
        rejected.reason.contains("violates denied CPU set") && rejected.reason.contains("overlap=1")
    }));
}

#[test]
fn generated_profile_plan_rejects_render_mask_below_minimum() {
    let topology = fake_topology_4c8t();
    let policy = GeneratedCpuSetPolicy {
        allowed_cpus: None,
        denied_cpus: None,
        min_render_cpus: 2,
        min_game_cpus: 1,
        min_compositor_cpus: 1,
        min_background_cpus: 2,
    };

    let plan = generate_topology_aware_profiles_with_policy(&topology, &policy);

    assert!(plan.rejected.iter().any(|rejected| {
        rejected.profile_name == "game-isolate-render"
            && rejected
                .reason
                .contains("render/main game work fewer than minimum CPUs")
    }));
}

#[test]
fn generated_profile_plan_rejects_background_helper_single_cpu_overload() {
    let topology = fake_topology_4c8t();
    let policy = GeneratedCpuSetPolicy {
        allowed_cpus: None,
        denied_cpus: None,
        min_render_cpus: 1,
        min_game_cpus: 1,
        min_compositor_cpus: 1,
        min_background_cpus: 7,
    };

    let plan = generate_topology_aware_profiles_with_policy(&topology, &policy);

    assert!(plan.rejected.iter().any(|rejected| {
        rejected
            .reason
            .contains("background/helper work onto too few CPUs")
    }));
}

#[test]
fn generated_profile_validation_rejects_offline_cpu_masks() {
    let topology = fake_topology_4c8t();
    let profile = Profile {
        name: "bad-offline".to_owned(),
        rules: vec![ProfileRule {
            affinity: Some(crate::affinity::CpuMask::parse("0,99").unwrap()),
            nice: None,
            ionice: None,
            match_class: vec![TaskClass::Game],
            match_comm: Vec::new(),
        }],
    };

    let err = validate_generated_profile(&profile, &topology, &GeneratedCpuSetPolicy::default())
        .unwrap_err();

    assert!(err.contains("requests offline CPUs"));
    assert!(err.contains("requested=0,99"));
    assert!(err.contains("online=0-7"));
}

#[test]
fn generated_profile_validation_rejects_empty_masks() {
    let topology = fake_topology_4c8t();
    let profile = Profile {
        name: "bad-empty".to_owned(),
        rules: vec![ProfileRule {
            affinity: Some(
                crate::affinity::CpuMask::parse("")
                    .unwrap_or_else(|_| crate::affinity::CpuMask::parse("0").unwrap()),
            ),
            nice: None,
            ionice: None,
            match_class: vec![TaskClass::Game],
            match_comm: Vec::new(),
        }],
    };

    if profile.rules[0].affinity.as_ref().unwrap().is_empty() {
        let err =
            validate_generated_profile(&profile, &topology, &GeneratedCpuSetPolicy::default())
                .unwrap_err();
        assert!(err.contains("empty CPU mask"));
    } else {
        assert!(!profile.rules[0].affinity.as_ref().unwrap().is_empty());
    }
}

#[test]
fn generated_profile_validation_rejects_compositor_zero_cpu_equivalent_empty_mask() {
    let topology = fake_topology_4c8t();
    let policy = GeneratedCpuSetPolicy {
        allowed_cpus: Some(crate::affinity::CpuMask::parse("0").unwrap()),
        denied_cpus: Some(crate::affinity::CpuMask::parse("0").unwrap()),
        min_render_cpus: 1,
        min_game_cpus: 1,
        min_compositor_cpus: 1,
        min_background_cpus: 2,
    };

    let plan = generate_topology_aware_profiles_with_policy(&topology, &policy);

    assert!(plan.rejected.iter().any(|rejected| {
        rejected.reason.contains("violates denied CPU set")
            || rejected.reason.contains("violates allowed CPU set")
    }));
}

#[test]
fn generated_profile_validation_rejects_audio_realtime_and_input_targets() {
    let topology = fake_topology_4c8t();
    let profile = Profile {
        name: "bad-critical".to_owned(),
        rules: vec![ProfileRule {
            affinity: Some(crate::affinity::CpuMask::parse("0").unwrap()),
            nice: None,
            ionice: None,
            match_class: vec![TaskClass::AudioRealtime, TaskClass::Input],
            match_comm: Vec::new(),
        }],
    };

    let err = validate_generated_profile(&profile, &topology, &GeneratedCpuSetPolicy::default())
        .unwrap_err();

    assert!(err.contains("critical realtime/input"));
}

#[test]
fn valid_generated_profile_plan_keeps_safe_templates_and_reports_rejections() {
    let topology = fake_topology_4c8t();
    let plan =
        generate_topology_aware_profiles_with_policy(&topology, &GeneratedCpuSetPolicy::default());
    let names = plan
        .profiles
        .iter()
        .map(|profile| profile.name.as_str())
        .collect::<Vec<_>>();

    assert!(plan.rejected.is_empty());
    assert_eq!(
        names,
        vec![
            "baseline-online",
            "game-isolate-render",
            "game-compositor-separate",
            "helper-spread",
            "wine-server-dedicated",
            "avoid-smt-contention"
        ]
    );
}

#[test]
fn topology_aware_candidate_plan_separates_baseline_recovery_from_optimization() {
    let topology = fake_topology_4c8t();
    let plan = generate_topology_aware_profile_candidate_plan(
        &topology,
        1234,
        &GeneratedCpuSetPolicy::default(),
    );

    assert_eq!(
        plan.recovery_fallback
            .as_ref()
            .map(CandidateAction::profile_name),
        Some("baseline-online")
    );
    assert!(
        plan.optimization_candidates
            .iter()
            .all(|candidate| candidate.profile_name() != "baseline-online")
    );
    assert!(
        plan.optimization_candidates
            .iter()
            .any(|candidate| candidate.profile_name() == "game-isolate-render")
    );
    assert!(plan.rejected.is_empty());
}
