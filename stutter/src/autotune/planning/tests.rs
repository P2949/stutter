//! Autotune candidate planning tests.

use super::{
    candidate::*, dry_run::*, executable_plan::*, plan_io::*, profile_candidates::*, suggestion::*,
};

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, BTreeSet},
        fs,
        path::{Path, PathBuf},
    };

    use super::*;
    use crate::{
        actions::{
            ActionState, ActionWarning, SafetyClass, TaskIdentity,
            irq_affinity::{IrqAffinityAction, IrqAffinityEvidence, IrqAffinityRisk},
            nice::{NiceAction, NicePolicy},
        },
        affinity::CpuMask,
        autotune::{conflicts::ActionConflictGroup, objective::ObjectiveKind},
        daemon_policy::{ActionDescriptor, ActionEffectScope, DaemonMode, RollbackRequirement},
        process_tree::TaskClass,
        profiles::{Profile, ProfileRule},
        topology::{CoreInfo, CpuInfo, TopologyModel},
    };

    fn fake_topology_4c8t() -> TopologyModel {
        let online_cpus = CpuMask::parse("0-7").unwrap();

        TopologyModel {
            online_cpus,
            cpus: vec![
                fake_cpu(0, 0, 0, 0, 5000),
                fake_cpu(1, 1, 0, 0, 4900),
                fake_cpu(2, 2, 0, 0, 4800),
                fake_cpu(3, 3, 0, 0, 4700),
                fake_cpu(4, 0, 0, 0, 5000),
                fake_cpu(5, 1, 0, 0, 4900),
                fake_cpu(6, 2, 0, 0, 4800),
                fake_cpu(7, 3, 0, 0, 4700),
            ],
            cores: vec![
                fake_core(0, 0, 0, vec![0, 4], 5000),
                fake_core(1, 0, 0, vec![1, 5], 4900),
                fake_core(2, 0, 0, vec![2, 6], 4800),
                fake_core(3, 0, 0, vec![3, 7], 4700),
            ],
            smt_siblings: BTreeMap::from([
                (0, vec![0, 4]),
                (4, vec![0, 4]),
                (1, vec![1, 5]),
                (5, vec![1, 5]),
                (2, vec![2, 6]),
                (6, vec![2, 6]),
                (3, vec![3, 7]),
                (7, vec![3, 7]),
            ]),
            numa_nodes: BTreeMap::from([(0, vec![0, 1, 2, 3, 4, 5, 6, 7])]),
            packages: BTreeMap::from([(0, vec![0, 1, 2, 3, 4, 5, 6, 7])]),
        }
    }

    fn fake_cpu(cpu: u32, core_id: u32, package_id: u32, numa_node: u32, max_mhz: u64) -> CpuInfo {
        CpuInfo {
            cpu,
            core_id: Some(core_id),
            package_id: Some(package_id),
            numa_node: Some(numa_node),
            max_mhz: Some(max_mhz),
            is_online: true,
        }
    }

    fn fake_core(
        core_id: u32,
        package_id: u32,
        numa_node: u32,
        cpus: Vec<u32>,
        max_mhz: u64,
    ) -> CoreInfo {
        CoreInfo {
            core_id: Some(core_id),
            package_id: Some(package_id),
            numa_node: Some(numa_node),
            cpus,
            max_mhz: Some(max_mhz),
            is_online: true,
        }
    }

    fn profile_by_name<'a>(profiles: &'a [Profile], name: &str) -> &'a Profile {
        profiles
            .iter()
            .find(|profile| profile.name == name)
            .unwrap_or_else(|| panic!("missing generated profile {name}"))
    }

    fn first_rule_for_class(profile: &Profile, class: TaskClass) -> &ProfileRule {
        profile
            .rules
            .iter()
            .find(|rule| rule.match_class.contains(&class))
            .unwrap_or_else(|| panic!("missing rule for {class:?} in {}", profile.name))
    }

    fn affinity(rule: &ProfileRule) -> &CpuMask {
        rule.affinity.as_ref().unwrap()
    }

    fn profile(name: &str) -> Profile {
        Profile {
            name: name.to_owned(),
            rules: vec![ProfileRule {
                affinity: Some(CpuMask::parse("0").unwrap()),
                nice: None,
                ionice: None,
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
            rejected.reason.contains("violates denied CPU set")
                && rejected.reason.contains("overlap=1")
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

        let err =
            validate_generated_profile(&profile, &topology, &GeneratedCpuSetPolicy::default())
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

        let err =
            validate_generated_profile(&profile, &topology, &GeneratedCpuSetPolicy::default())
                .unwrap_err();

        assert!(err.contains("critical realtime/input"));
    }

    #[test]
    fn valid_generated_profile_plan_keeps_safe_templates_and_reports_rejections() {
        let topology = fake_topology_4c8t();
        let plan = generate_topology_aware_profiles_with_policy(
            &topology,
            &GeneratedCpuSetPolicy::default(),
        );
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
    fn suggestion_from_dry_run_record_renders_requested_shape() {
        let record = CandidateDryRunRecord {
            candidate_name: "game-main-suggested".to_owned(),
            affected_tasks: 31,
            warnings: Vec::new(),
            safety_class: SafetyClass::ReversibleLowRisk,
            eligible: true,
            reason: None,
        };

        let suggestion = suggestion_from_dry_run_record(
            &record,
            1234,
            None,
            SafetyClass::ReversibleLowRisk,
            "scheduler pressure detected on Game/WineServer classes",
        )
        .unwrap();

        let rendered = render_candidate_suggestion(&suggestion);

        assert!(rendered.contains("candidate=game-main-suggested"));
        assert!(rendered.contains("action=cpu-affinity-profile"));
        assert!(rendered.contains("affected_tasks=31"));
        assert!(rendered.contains("safety=ReversibleLowRisk"));
        assert!(
            rendered.contains("reason=\"scheduler pressure detected on Game/WineServer classes\"")
        );
        assert!(rendered.contains("note=\"suggest mode did not apply this change\""));
        assert!(rendered.contains("required_mode=apply-low-risk"));
        assert!(rendered.contains("dry_run_command=\"stutter apply-profile --tree-pid 1234 --profile <generated-or-existing-profile> --dry-run\""));
        assert!(rendered.contains("manual_apply_command=\"stutter apply-profile --tree-pid 1234 --profile <generated-or-existing-profile>\""));
    }

    #[test]
    fn generic_candidate_suggestion_writes_plan_file_and_uses_apply_candidate_command() {
        let plan_dir = temp_candidate_plan_dir("generic-nice");
        let candidate = CandidateAction::Nice {
            plan: NiceActionPlan {
                name: "nice-browser-helper".to_owned(),
                action: NiceAction {
                    targets: vec![TaskIdentity {
                        tid: 1234,
                        process_pid: Some(1234),
                        comm: Some("browser".to_owned()),
                        starttime_ticks: Some(77),
                    }],
                    nice: 5,
                    policy: NicePolicy::default(),
                },
                target_root_pid: Some(1234),
                evidence: vec![CandidateEvidence::new("cpu_pressure", "high", 0.9)],
                objective: ObjectiveKind::DesktopInteractivity,
            },
        };
        let records = vec![CandidateDryRunRecord {
            candidate_name: candidate.candidate_name().to_owned(),
            affected_tasks: 1,
            warnings: Vec::new(),
            safety_class: candidate.safety_class(),
            eligible: true,
            reason: None,
        }];

        let suggestions = suggestions_from_candidates_and_dry_run_records(
            std::slice::from_ref(&candidate),
            &records,
            &plan_dir,
            None,
            SafetyClass::ReversibleMediumRisk,
            "compile CPU pressure",
        )
        .unwrap();

        assert_eq!(suggestions.len(), 1);
        let suggestion = &suggestions[0];
        let plan_path = candidate_plan_path(&candidate, &plan_dir);

        assert!(plan_path.exists());
        assert_eq!(suggestion.candidate_name, "nice-browser-helper");
        assert_eq!(suggestion.action_kind, "nice");
        assert_eq!(suggestion.objective, ObjectiveKind::DesktopInteractivity);
        assert_eq!(suggestion.evidence.len(), 1);
        assert_eq!(suggestion.required_mode, DaemonMode::ApplyMediumRisk);
        assert_eq!(
            suggestion.required_safety_class,
            SafetyClass::ReversibleMediumRisk
        );
        assert_eq!(
            suggestion.dry_run_command.as_deref(),
            Some(format!(
                "stutter autotune apply-candidate --candidate-json {} --dry-run",
                plan_path.display()
            ))
            .as_deref()
        );
        assert_eq!(
            suggestion.manual_apply_command.as_deref(),
            Some(format!(
                "stutter autotune apply-candidate --candidate-json {}",
                plan_path.display()
            ))
            .as_deref()
        );

        let decoded: CandidatePlanFile =
            serde_json::from_slice(&fs::read(&plan_path).unwrap()).unwrap();
        assert_eq!(decoded.candidate.candidate_name, "nice-browser-helper");
        assert_eq!(decoded.candidate.action_kind, "nice");
        assert!(decoded.executable.is_some());

        let rendered = render_candidate_suggestion(suggestion);
        assert!(rendered.contains("action=nice"));
        assert!(rendered.contains("action_kind=nice"));
        assert!(rendered.contains("dry_run_command=\"stutter autotune apply-candidate"));
        assert!(rendered.contains("manual_apply_command=\"stutter autotune apply-candidate"));
    }

    #[test]
    fn high_risk_system_candidate_suggestion_is_dry_run_only() {
        let plan_dir = temp_candidate_plan_dir("high-risk-irq");
        let candidate = CandidateAction::IrqAffinity {
            plan: IrqAffinityActionPlan {
                name: "irq-affinity-44-high-risk".to_owned(),
                action: IrqAffinityAction::new(
                    44,
                    "gpu".to_owned(),
                    "2".to_owned(),
                    IrqAffinityRisk::HighRisk,
                    IrqAffinityEvidence {
                        strong_irq_evidence: true,
                        stable_irq_identity: false,
                        known_device_mapping: true,
                        observed_irq: Some(44),
                        observed_device_hint: Some("gpu".to_owned()),
                        reason: "test IRQ pressure".to_owned(),
                    },
                ),
                evidence: vec![CandidateEvidence::new("irq", "gpu", 0.8)],
                objective: ObjectiveKind::IrqOverlapReduction,
            },
        };
        let records = vec![CandidateDryRunRecord {
            candidate_name: candidate.candidate_name().to_owned(),
            affected_tasks: 1,
            warnings: Vec::new(),
            safety_class: candidate.safety_class(),
            eligible: true,
            reason: None,
        }];

        let suggestions = suggestions_from_candidates_and_dry_run_records(
            std::slice::from_ref(&candidate),
            &records,
            &plan_dir,
            None,
            SafetyClass::HighRisk,
            "IRQ overlap detected",
        )
        .unwrap();

        assert_eq!(suggestions.len(), 1);
        let suggestion = &suggestions[0];
        let plan_path = candidate_plan_path(&candidate, &plan_dir);

        assert!(plan_path.exists());
        assert_eq!(suggestion.action_kind, "irq_affinity");
        assert_eq!(suggestion.required_mode, DaemonMode::ApplyHighRisk);
        assert_eq!(suggestion.required_safety_class, SafetyClass::HighRisk);
        assert!(suggestion.dry_run_command.is_some());
        assert_eq!(suggestion.manual_apply_command, None);
        assert!(
            suggestion
                .manual_only_reason
                .as_deref()
                .unwrap_or_default()
                .contains("manual-only high-risk/system-adjacent")
        );

        let dry_run_plan = apply_candidate_plan_file(&plan_path, true).unwrap();
        assert_eq!(
            dry_run_plan.candidate.candidate_name,
            "irq-affinity-44-high-risk"
        );
        assert!(dry_run_plan.executable.is_none());

        let err = apply_candidate_plan_file(&plan_path, false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("manual_only_high_risk"));
    }

    #[test]
    fn cpu_affinity_suggestion_preserves_apply_profile_and_does_not_write_plan_file() {
        let plan_dir = temp_candidate_plan_dir("cpu-affinity-preserve-apply-profile");
        let profile = Profile {
            name: "game".to_owned(),
            rules: Vec::new(),
        };
        let candidate = CandidateAction::CpuAffinityProfile {
            plan: CpuAffinityProfilePlan {
                profile_name: "game".to_owned(),
                profile,
                tree_pid: 1234,
            },
        };
        let records = vec![CandidateDryRunRecord {
            candidate_name: candidate.candidate_name().to_owned(),
            affected_tasks: 1,
            warnings: Vec::new(),
            safety_class: candidate.safety_class(),
            eligible: true,
            reason: None,
        }];
        let profile_path = Path::new("/tmp/profile.toml");

        let suggestions = suggestions_from_candidates_and_dry_run_records(
            std::slice::from_ref(&candidate),
            &records,
            &plan_dir,
            Some(profile_path),
            SafetyClass::ReversibleMediumRisk,
            "scheduler pressure",
        )
        .unwrap();

        assert_eq!(suggestions.len(), 1);
        let suggestion = &suggestions[0];
        assert_eq!(suggestion.action_kind, "cpu_affinity_profile");
        assert_eq!(
            suggestion.dry_run_command.as_deref(),
            Some("stutter apply-profile --tree-pid 1234 --profile /tmp/profile.toml --dry-run")
        );
        assert_eq!(
            suggestion.manual_apply_command.as_deref(),
            Some("stutter apply-profile --tree-pid 1234 --profile /tmp/profile.toml")
        );
        assert!(!candidate_plan_path(&candidate, &plan_dir).exists());
    }

    #[test]
    fn candidate_plan_file_can_embed_executable_process_local_payload() {
        let candidate = CandidateAction::Nice {
            plan: NiceActionPlan {
                name: "nice-browser-helper".to_owned(),
                action: NiceAction {
                    targets: vec![TaskIdentity {
                        tid: 1234,
                        process_pid: Some(1234),
                        comm: Some("browser".to_owned()),
                        starttime_ticks: Some(77),
                    }],
                    nice: 5,
                    policy: NicePolicy::default(),
                },
                target_root_pid: Some(1234),
                evidence: vec![CandidateEvidence::new("cpu_pressure", "high", 0.9)],
                objective: ObjectiveKind::DesktopInteractivity,
            },
        };

        let plan = CandidatePlanFile::from_candidate(&candidate, Some(1));
        let json = serde_json::to_string(&plan).unwrap();
        let decoded: CandidatePlanFile = serde_json::from_str(&json).unwrap();

        assert!(decoded.executable.is_some());
        let decoded_candidate = decoded.executable.unwrap().into_candidate();
        assert_eq!(decoded_candidate.action_kind(), "nice");
        assert_eq!(decoded_candidate.candidate_name(), "nice-browser-helper");
    }

    #[test]
    fn cpu_affinity_candidate_plan_file_is_manual_only_with_stable_rejection() {
        let plan_dir = temp_candidate_plan_dir("cpu-affinity-plan-manual-only");
        let candidate = CandidateAction::CpuAffinityProfile {
            plan: CpuAffinityProfilePlan {
                profile_name: "game".to_owned(),
                profile: Profile {
                    name: "game".to_owned(),
                    rules: vec![ProfileRule {
                        affinity: Some(CpuMask::parse("0").unwrap()),
                        nice: None,
                        ionice: None,
                        match_class: vec![TaskClass::Game],
                        match_comm: Vec::new(),
                    }],
                },
                tree_pid: 1234,
            },
        };
        let plan_path = candidate_plan_path(&candidate, &plan_dir);

        let plan = write_candidate_plan_file(&plan_path, &candidate, Some(1)).unwrap();
        assert!(plan.executable.is_none());
        assert_eq!(
            plan.manual_apply_command.as_deref(),
            Some("stutter apply-profile --tree-pid 1234 --profile <generated-or-existing-profile>")
        );
        assert_eq!(
            plan.manual_only_reason.as_deref(),
            Some("cpu-affinity profiles use apply-profile, not candidate-plan apply")
        );

        let decoded: CandidatePlanFile =
            serde_json::from_slice(&std::fs::read(&plan_path).unwrap()).unwrap();
        assert!(decoded.executable.is_none());
        assert_eq!(decoded.manual_only_reason, plan.manual_only_reason);

        let err = apply_candidate_plan_file(&plan_path, false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("candidate_plan_manual_only"));
        assert!(err.contains("apply-profile"));
    }

    #[test]
    fn suggestion_from_dry_run_record_uses_existing_profile_path_when_available() {
        let record = CandidateDryRunRecord {
            candidate_name: "game-main-suggested".to_owned(),
            affected_tasks: 31,
            warnings: Vec::new(),
            safety_class: SafetyClass::ReversibleLowRisk,
            eligible: true,
            reason: None,
        };

        let suggestion = suggestion_from_dry_run_record(
            &record,
            1234,
            Some(Path::new("/tmp/profiles.toml")),
            SafetyClass::ReversibleLowRisk,
            "scheduler pressure detected on Game/WineServer classes",
        )
        .unwrap();

        assert_eq!(
            suggestion.dry_run_command.as_deref(),
            Some("stutter apply-profile --tree-pid 1234 --profile /tmp/profiles.toml --dry-run")
        );
        assert_eq!(
            suggestion.manual_apply_command.as_deref(),
            Some("stutter apply-profile --tree-pid 1234 --profile /tmp/profiles.toml")
        );
    }

    #[test]
    fn suggestion_from_dry_run_record_skips_ineligible_candidate() {
        let record = CandidateDryRunRecord {
            candidate_name: "bad-candidate".to_owned(),
            affected_tasks: 0,
            warnings: Vec::new(),
            safety_class: SafetyClass::ReversibleLowRisk,
            eligible: false,
            reason: Some("dry-run matched zero affected tasks".to_owned()),
        };

        let suggestion = suggestion_from_dry_run_record(
            &record,
            1234,
            None,
            SafetyClass::ReversibleLowRisk,
            "scheduler pressure detected on Game/WineServer classes",
        );

        assert!(suggestion.is_none());
    }

    #[test]
    fn render_candidate_suggestion_escapes_reason_and_commands() {
        let suggestion = CandidateSuggestion {
            candidate_name: "candidate with space".to_owned(),
            action_kind: "cpu_affinity_profile".to_owned(),
            descriptor: ActionDescriptor {
                action_id: crate::actions::ActionId::new(
                    "cpu-affinity-profile:candidate with space".to_owned(),
                ),
                action_kind: "cpu_affinity_profile".to_owned(),
                safety_class: SafetyClass::ReversibleLowRisk,
                effect_scope: ActionEffectScope::LocalProcessTree,
                rollback: RollbackRequirement::RequiredBeforeApply,
                persistent_effect: false,
                touches_system_wide_state: false,
                requires_explicit_target: true,
                confidence: None,
            },
            objective: ObjectiveKind::StutterScore,
            evidence: Vec::new(),
            affected_tasks: 31,
            safety: SafetyClass::ReversibleLowRisk,
            reason: "scheduler \"pressure\"\nnext".to_owned(),
            dry_run_command: Some(
                "stutter apply-profile --tree-pid 1234 --profile /tmp/profile \"quoted\".toml --dry-run"
                    .to_owned(),
            ),
            manual_apply_command: Some(
                "stutter apply-profile --tree-pid 1234 --profile /tmp/profile \"quoted\".toml"
                    .to_owned(),
            ),
            required_mode: DaemonMode::ApplyLowRisk,
            required_safety_class: SafetyClass::ReversibleLowRisk,
            manual_only_reason: None,
        };

        let rendered = render_candidate_suggestion(&suggestion);

        assert!(rendered.contains("candidate=\"candidate with space\""));
        assert!(rendered.contains("reason=\"scheduler \\\"pressure\\\"\\nnext\""));
        assert!(rendered.contains("dry_run_command=\"stutter apply-profile --tree-pid 1234 --profile /tmp/profile \\\"quoted\\\".toml --dry-run\""));
        assert!(rendered.contains("manual_apply_command=\"stutter apply-profile --tree-pid 1234 --profile /tmp/profile \\\"quoted\\\".toml\""));
    }

    fn temp_candidate_plan_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "stutter-candidate-plan-{name}-{}",
            crate::audit::unix_nanos_now()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn generate_profile_candidates_for_observation_without_target_pid_returns_no_candidates() {
        let profiles = vec![Profile {
            name: "fixture-game-helper".to_owned(),
            rules: vec![ProfileRule {
                affinity: Some(CpuMask::parse("0").unwrap()),
                nice: None,
                ionice: None,
                match_class: vec![TaskClass::Game],
                match_comm: Vec::new(),
            }],
        }];

        let observation = crate::autotune::observation::AutotuneObservation {
            target_root_pid: None,
            active_tasks: vec![crate::autotune::observation::ActiveTaskSnapshot {
                tid: 1234,
                process_pid: 1234,
                comm: "game-main".to_owned(),
                class: TaskClass::Game,
                process_starttime_ticks: Some(10),
                task_starttime_ticks: Some(1234),
                cgroup_path: Some("/user.slice/fixture.scope".to_owned()),
            }],
            ..crate::autotune::observation::AutotuneObservation::default()
        };

        let plan = generate_profile_candidate_plan_for_observation(&profiles, &observation);

        assert!(plan.optimization_candidates.is_empty());
        assert!(plan.recovery_fallback.is_none());
        assert!(plan.rejected.is_empty());
    }

    fn eligible_record(name: &str, affected_tasks: usize) -> CandidateDryRunRecord {
        CandidateDryRunRecord {
            candidate_name: name.to_owned(),
            affected_tasks,
            warnings: Vec::new(),
            safety_class: SafetyClass::ReversibleLowRisk,
            eligible: true,
            reason: None,
        }
    }

    #[derive(Default)]
    struct FakeDryRunner {
        dry_run_calls: usize,
        apply_calls: usize,
    }

    impl CandidateDryRunner for FakeDryRunner {
        fn dry_run(&mut self, candidate: &CandidateAction) -> CandidateDryRunRecord {
            self.dry_run_calls += 1;
            eligible_record(candidate.profile_name(), 31)
        }
    }

    #[test]
    fn suggest_mode_emits_candidates_but_never_calls_apply() {
        let candidates = vec![CandidateAction::cpu_affinity_profile(
            profile("game-main-suggested"),
            1234,
        )];
        let mut runner = FakeDryRunner::default();

        let records = dry_run_candidates_with_runner(&candidates, &mut runner);
        let suggestions = suggestions_from_dry_run_records(
            &records,
            1234,
            None,
            SafetyClass::ReversibleLowRisk,
            "scheduler pressure detected on Game/WineServer classes",
        );

        assert_eq!(runner.dry_run_calls, 1);
        assert_eq!(runner.apply_calls, 0);
        assert_eq!(suggestions.len(), 1);

        let rendered = render_candidate_suggestion(&suggestions[0]);
        assert!(rendered.contains("candidate=game-main-suggested"));
        assert!(rendered.contains("note=\"suggest mode did not apply this change\""));
        assert!(rendered.contains("dry_run_command=\"stutter apply-profile --tree-pid 1234 --profile <generated-or-existing-profile> --dry-run\""));
    }

    #[test]
    fn profile_with_zero_affected_tasks_is_rejected() {
        let record = CandidateDryRunRecord {
            candidate_name: "zero-task-profile".to_owned(),
            affected_tasks: 0,
            warnings: Vec::new(),
            safety_class: SafetyClass::ReversibleLowRisk,
            eligible: false,
            reason: Some("dry-run matched zero affected tasks".to_owned()),
        };

        let suggestion = suggestion_from_dry_run_record(
            &record,
            1234,
            None,
            SafetyClass::ReversibleLowRisk,
            "scheduler pressure detected on Game/WineServer classes",
        );

        assert!(suggestion.is_none());
        assert!(!record.eligible);
        assert_eq!(
            record.reason.as_deref(),
            Some("dry-run matched zero affected tasks")
        );
    }

    #[test]
    fn profile_dry_run_warning_is_preserved() {
        let state = ActionState {
            applied: false,
            affected_tasks: 31,
            checked_tasks: 31,
            pending_changes: 31,
            warnings: vec![ActionWarning {
                message: "restore file already exists at /tmp/stutter-restore.json; new affinity records will be merged".to_owned(),
            }],
        };

        let record = dry_run_record_from_action_state(
            "warned-profile".to_owned(),
            SafetyClass::ReversibleLowRisk,
            state,
        );

        assert!(record.eligible);
        assert_eq!(record.affected_tasks, 31);
        assert_eq!(record.warnings.len(), 1);
        assert!(
            record.warnings[0]
                .message
                .contains("restore file already exists")
        );
    }

    #[test]
    fn high_risk_candidates_are_blocked() {
        let record = CandidateDryRunRecord {
            candidate_name: "high-risk-profile".to_owned(),
            affected_tasks: 31,
            warnings: Vec::new(),
            safety_class: SafetyClass::HighRisk,
            eligible: true,
            reason: None,
        };

        let suggestion = suggestion_from_dry_run_record(
            &record,
            1234,
            None,
            SafetyClass::ReversibleLowRisk,
            "scheduler pressure detected on Game/WineServer classes",
        );

        assert!(suggestion.is_none());
    }

    #[test]
    fn high_risk_candidates_are_allowed_when_policy_allows_high_risk() {
        let record = CandidateDryRunRecord {
            candidate_name: "high-risk-profile".to_owned(),
            affected_tasks: 31,
            warnings: Vec::new(),
            safety_class: SafetyClass::HighRisk,
            eligible: true,
            reason: None,
        };

        let suggestion = suggestion_from_dry_run_record(
            &record,
            1234,
            None,
            SafetyClass::HighRisk,
            "scheduler pressure detected on Game/WineServer classes",
        );

        assert!(suggestion.is_some());
        assert_eq!(suggestion.unwrap().safety, SafetyClass::HighRisk);
    }

    #[test]
    fn dry_run_candidate_records_failure_as_ineligible() {
        let candidate = CandidateAction::cpu_affinity_profile(profile("bad-tree"), 0);

        let record = dry_run_candidate(&candidate);

        assert_eq!(record.candidate_name, "bad-tree");
        assert_eq!(record.affected_tasks, 0);
        assert_eq!(record.safety_class, SafetyClass::ReversibleLowRisk);
        assert!(!record.eligible);
        assert!(
            record
                .reason
                .as_deref()
                .unwrap_or_default()
                .contains("dry-run failed")
        );
        assert!(
            record
                .reason
                .as_deref()
                .unwrap_or_default()
                .contains("tree pid must be greater than zero")
        );
    }

    #[test]
    fn dry_run_candidates_preserves_candidate_order() {
        let candidates = vec![
            CandidateAction::cpu_affinity_profile(profile("first"), 0),
            CandidateAction::cpu_affinity_profile(profile("second"), 0),
        ];

        let records = dry_run_candidates(&candidates);

        assert_eq!(records.len(), 2);
        assert_eq!(records[0].candidate_name, "first");
        assert_eq!(records[1].candidate_name, "second");
    }

    #[test]
    fn candidate_helpers_return_stable_metadata() {
        let candidate = CandidateAction::cpu_affinity_profile(profile("game-main"), 1234);

        assert_eq!(candidate.candidate_name(), "game-main");
        assert_eq!(candidate.target_root_pid(), Some(1234));
        assert_eq!(candidate.action_kind(), "cpu_affinity_profile");
        assert_eq!(candidate.safety_class(), SafetyClass::ReversibleLowRisk);
        assert_eq!(
            candidate.descriptor().effect_scope,
            ActionEffectScope::LocalProcessTree
        );
        assert_eq!(
            candidate.conflict_group(),
            ActionConflictGroup::CpuPlacement
        );
    }

    #[test]
    fn generic_candidate_variant_reports_descriptor_scope_and_objective() {
        let candidate = CandidateAction::Nice {
            plan: NiceActionPlan {
                name: "nice-root-1234-to-5".to_owned(),
                action: crate::actions::nice::NiceAction {
                    targets: vec![crate::actions::TaskIdentity {
                        tid: 1234,
                        process_pid: Some(1234),
                        comm: None,
                        starttime_ticks: None,
                    }],
                    nice: 5,
                    policy: crate::actions::nice::NicePolicy::default(),
                },
                target_root_pid: Some(1234),
                evidence: vec![CandidateEvidence::new("situation", "CompileCpuBound", 0.8)],
                objective: ObjectiveKind::DesktopInteractivity,
            },
        };

        assert_eq!(candidate.candidate_name(), "nice-root-1234-to-5");
        assert_eq!(candidate.action_kind(), "nice");
        assert_eq!(candidate.safety_class(), SafetyClass::ReversibleMediumRisk);
        assert_eq!(
            candidate.effect_scope(),
            ActionEffectScope::LocalProcessTree
        );
        assert_eq!(candidate.target_root_pid(), Some(1234));
        assert_eq!(candidate.conflict_group(), ActionConflictGroup::CpuPriority);
        assert_eq!(candidate.objective(), ObjectiveKind::DesktopInteractivity);
    }

    #[test]
    fn fake_candidate_uses_candidate_plan_metadata() {
        let candidate = CandidateAction::fake(
            crate::actions::ActionId::new("fake-plan".to_owned()),
            SafetyClass::ReversibleMediumRisk,
        );

        assert_eq!(candidate.candidate_name(), "fake-profile");
        assert_eq!(candidate.action_kind(), "fake");
        assert_eq!(candidate.target_root_pid(), None);
        assert_eq!(candidate.action_id().as_str(), "fake-plan");
        assert_eq!(candidate.safety_class(), SafetyClass::ReversibleMediumRisk);
        assert_eq!(candidate.effect_scope(), ActionEffectScope::ObserveOnly);
        assert!(candidate.evidence().is_empty());
        assert_eq!(candidate.objective(), ObjectiveKind::StutterScore);
        assert_eq!(candidate.conflict_group(), ActionConflictGroup::None);
        assert_eq!(candidate.describe(), "fake action fake-plan");
        assert!(!candidate.is_high_risk_system_adjacent());
        assert!(candidate.manual_only_reason().is_none());

        let high_risk_candidate = CandidateAction::fake(
            crate::actions::ActionId::new("fake-high-risk".to_owned()),
            SafetyClass::HighRisk,
        );

        assert!(high_risk_candidate.is_high_risk_system_adjacent());
        assert_eq!(
            high_risk_candidate.manual_only_reason(),
            Some(
                "manual-only high-risk/system-adjacent candidate; autonomous apply is disabled for action_kind=fake"
                    .to_owned()
            )
        );
    }

    #[test]
    fn apply_candidate_requires_successful_eligibility_promotion() {
        let candidate = CandidateAction::cpu_affinity_profile(profile("game-main"), 1234);

        let apply_candidate =
            try_promote_to_apply_candidate(candidate.clone(), ApplyEligibility::approved())
                .unwrap();
        assert_eq!(
            apply_candidate.candidate().candidate_name(),
            candidate.candidate_name()
        );

        let denied =
            try_promote_to_apply_candidate(candidate, ApplyEligibility::denied("policy denied"))
                .unwrap_err();
        assert_eq!(denied.denial_message(), "policy denied");
    }

    #[test]
    fn profile_with_nice_or_ionice_is_medium_risk_candidate() {
        let candidate = CandidateAction::cpu_affinity_profile(
            Profile {
                name: "background-demotion".to_owned(),
                rules: vec![ProfileRule {
                    affinity: None,
                    nice: Some(10),
                    ionice: Some(crate::actions::ioprio::IoPrioValue::idle()),
                    match_class: vec![TaskClass::Indexer],
                    match_comm: Vec::new(),
                }],
            },
            1234,
        );

        assert_eq!(candidate.safety_class(), SafetyClass::ReversibleMediumRisk);
    }

    fn dry_run_record(safety_class: SafetyClass) -> CandidateDryRunRecord {
        CandidateDryRunRecord {
            candidate_name: "game-main".to_owned(),
            affected_tasks: 4,
            warnings: Vec::new(),
            safety_class,
            eligible: true,
            reason: None,
        }
    }

    #[test]
    fn low_risk_suggestion_renders_policy_aware_commands() {
        let suggestion = suggestion_from_dry_run_record(
            &dry_run_record(SafetyClass::ReversibleLowRisk),
            1234,
            Some(Path::new("profiles.toml")),
            SafetyClass::ReversibleLowRisk,
            "scheduler pressure detected",
        )
        .unwrap();

        assert_eq!(suggestion.required_mode, DaemonMode::ApplyLowRisk);
        assert_eq!(
            suggestion.required_safety_class,
            SafetyClass::ReversibleLowRisk
        );
        assert_eq!(
            suggestion.dry_run_command.as_deref(),
            Some("stutter apply-profile --tree-pid 1234 --profile profiles.toml --dry-run")
        );
        assert_eq!(
            suggestion.manual_apply_command.as_deref(),
            Some("stutter apply-profile --tree-pid 1234 --profile profiles.toml")
        );

        let rendered = render_candidate_suggestion(&suggestion);
        assert!(rendered.contains("suggest mode did not apply this change"));
        assert!(rendered.contains("required_mode=apply-low-risk"));
        assert!(rendered.contains("required_safety_class=ReversibleLowRisk"));
        assert!(rendered.contains("rollback=\"stutter restore\""));
        assert!(rendered.contains(
            "dry_run_command=\"stutter apply-profile --tree-pid 1234 --profile profiles.toml --dry-run\""
        ));
        assert!(rendered.contains(
            "manual_apply_command=\"stutter apply-profile --tree-pid 1234 --profile profiles.toml\""
        ));
    }

    #[test]
    fn medium_risk_suggestion_requires_medium_mode_and_flag() {
        let suggestion = suggestion_from_dry_run_record(
            &dry_run_record(SafetyClass::ReversibleMediumRisk),
            1234,
            Some(Path::new("profiles.toml")),
            SafetyClass::ReversibleMediumRisk,
            "priority profile may help",
        )
        .unwrap();

        assert_eq!(suggestion.required_mode, DaemonMode::ApplyMediumRisk);
        assert_eq!(
            suggestion.required_safety_class,
            SafetyClass::ReversibleMediumRisk
        );
        assert_eq!(
            suggestion.manual_apply_command.as_deref(),
            Some(
                "stutter apply-profile --tree-pid 1234 --profile profiles.toml --allow-medium-risk"
            )
        );

        let rendered = render_candidate_suggestion(&suggestion);
        assert!(rendered.contains("required_mode=apply-medium-risk"));
        assert!(rendered.contains("required_safety_class=ReversibleMediumRisk"));
        assert!(rendered.contains("--allow-medium-risk"));
    }

    #[test]
    fn high_risk_suggestion_suppresses_manual_apply_command() {
        let suggestion = suggestion_from_dry_run_record(
            &dry_run_record(SafetyClass::HighRisk),
            1234,
            Some(Path::new("profiles.toml")),
            SafetyClass::HighRisk,
            "high risk candidate",
        )
        .unwrap();

        assert_eq!(suggestion.required_mode, DaemonMode::ApplyHighRisk);
        assert_eq!(suggestion.required_safety_class, SafetyClass::HighRisk);
        assert_eq!(
            suggestion.dry_run_command.as_deref(),
            Some("stutter apply-profile --tree-pid 1234 --profile profiles.toml --dry-run")
        );
        assert_eq!(suggestion.manual_apply_command, None);

        let rendered = render_candidate_suggestion(&suggestion);
        assert!(rendered.contains("required_mode=apply-high-risk"));
        assert!(rendered.contains("manual_apply_command=none"));
        assert!(!rendered.contains("manual_apply_command=\"stutter apply-profile"));
    }
}
