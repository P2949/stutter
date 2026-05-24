//! Workload-policy planner tests extracted from `autotune::planner`.
//!
//! Owns workload-memory cooldown and planner golden-case fixture tests.
//! Does not own shared fixtures or production planner behavior.

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::super::{
        super::{CandidateDenyReason, CandidatePlanner, PlannerInput},
        support::*,
    };
    use crate::autotune::activity::ActivityLevel;

    #[test]
    fn workload_memory_cools_down_same_workload_without_blocking_other_workload() {
        let policy = policy(DaemonMode::Suggest);
        let candidate = nice_candidate();
        let mut controller_state = ControllerRuntimeState::default();
        let mut same_workload = observation();
        same_workload.primary_situation = SituationKind::CompileCpuBound;
        same_workload.focus_kind = Some(FocusGroupKind::Compile);
        same_workload.refresh_situation_classification();
        controller_state.record_candidate_result(
            crate::autotune::controller::ControllerCandidateResultInput {
                candidate: &candidate,
                observation: &same_workload,
                cpu_topology_signature: None,
                result: CandidateMemoryResult::Reverted,
                diagnostic_baseline_raw_score_total: Some(100),
                diagnostic_current_raw_score_total: Some(120),
                rollback_reason: Some("regressed".to_owned()),
                cooldown_expires_unix_nanos: Some(same_workload.now_unix_nanos + 10_000),
            },
        );

        let mut dry_runner = CountingDryRunner::default();
        let same_eval = evaluate_candidate_with_runner(
            &policy,
            &same_workload,
            &same_workload.capabilities,
            &controller_state,
            candidate.clone(),
            1.0,
            &mut dry_runner,
        );
        assert!(
            same_eval
                .deny_reasons
                .contains(&CandidateDenyReason::CooldownActive)
        );

        let mut other_workload = same_workload.clone();
        other_workload
            .workload_identity
            .as_mut()
            .unwrap()
            .stable_hash = "different-workload".to_owned();
        other_workload.workload_identity.as_mut().unwrap().exe_ino = Some(99);
        let mut dry_runner = CountingDryRunner::default();
        let other_eval = evaluate_candidate_with_runner(
            &policy,
            &other_workload,
            &other_workload.capabilities,
            &controller_state,
            candidate,
            1.0,
            &mut dry_runner,
        );

        assert!(
            !other_eval
                .deny_reasons
                .contains(&CandidateDenyReason::CooldownActive)
        );
    }

    #[derive(Debug, Deserialize)]
    struct PlannerGoldenCase {
        situation: String,
        focus_kind: String,
        policy: DaemonPolicyFixture,
        expected_selected_action_kind: Option<String>,
        expected_total_proposals: usize,
        expected_eligible_proposals: usize,
        expected_evaluations: Vec<ExpectedEvaluation>,
        #[serde(default)]
        low_data_quality: bool,
        #[serde(default)]
        critical_realtime: bool,
        #[serde(default)]
        cooldown_active: bool,
        #[serde(default)]
        kept_conflict: bool,
        #[serde(default)]
        external_mutation: bool,
        #[serde(default)]
        cpu_power_evidence: bool,
        #[serde(default)]
        gpu_power_evidence: bool,
        #[serde(default)]
        irq_evidence: bool,
        #[serde(default)]
        vm_evidence: bool,
        #[serde(default)]
        thermal_degraded: bool,
        #[serde(default)]
        activity_level: Option<ActivityLevel>,
        // Optional ObjectiveSignals override for fixtures that exercise the
        // rolling-window/observation signal path directly.
        #[serde(default)]
        hardware_signals: Option<serde_json::Value>,
    }

    #[derive(Debug, Default, Deserialize)]
    struct DaemonPolicyFixture {
        mode: String,
        #[serde(default)]
        allow_system_wide_suggestions: bool,
        #[serde(default)]
        allow_medium_risk_apply: bool,
        #[serde(default)]
        enabled_action_families: Vec<String>,
        #[serde(default)]
        irq_devices: Vec<String>,
        #[serde(default)]
        gpu_cards: Vec<String>,
        #[serde(default)]
        allow_gpu_power_in_autotune: bool,
        #[serde(default)]
        vm_knobs: Vec<String>,
        #[serde(default)]
        allow_vm_knobs_in_autotune: bool,
        #[serde(default)]
        autonomous_families: Vec<String>,
        #[serde(default)]
        compile_cgroup: Option<String>,
        #[serde(default)]
        background_cgroup: Option<String>,
        #[serde(default)]
        game_cgroup: Option<String>,
    }

    #[derive(Debug, Deserialize)]
    struct ExpectedEvaluation {
        action_kind: String,
        objective: String,
        eligible: bool,
        min_confidence: f32,
        max_confidence: f32,
        dry_run_affected_tasks: Option<usize>,
        manual_only: bool,
        deny_reason_codes: Vec<String>,
    }

    #[test]
    fn planner_golden_cases() {
        let fixture_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("testdata/autotune/planner");
        let mut paths = std::fs::read_dir(&fixture_dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
            .collect::<Vec<_>>();
        paths.sort();

        let expected_names = vec![
            "browser_cpu_pressure.json",
            "browser_focused.json",
            "browser_gpu_video.json",
            "browser_io_pressure.json",
            "browser_memory_pressure.json",
            "compile_cpu_bound.json",
            "compile_linker_pressure.json",
            "compositor_pressure.json",
            "cooldown_active.json",
            "critical_realtime_present.json",
            "external_mutation_detected.json",
            "game_cpu_scheduler_pressure.json",
            "game_gpu_bound.json",
            "game_gpu_power_limited.json",
            "game_gpu_profile_switch_medium_risk.json",
            "game_idle_suppressed.json",
            "game_irq_gpu_medium_risk.json",
            "game_irq_pressure_signals_present.json",
            "io_pressure.json",
            "irq_pressure.json",
            "kept_action_conflict.json",
            "low_data_quality.json",
            "media_playback.json",
            "memory_pressure_swappiness_medium_risk.json",
            "recording_active.json",
            "thermal_degraded.json",
            "virtual_machine_load.json",
        ];
        let actual_names = paths
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(actual_names, expected_names);

        for path in paths {
            let text = std::fs::read_to_string(&path).unwrap();
            let case: PlannerGoldenCase = serde_json::from_str(&text)
                .unwrap_or_else(|err| panic!("failed to parse {}: {err}", path.display()));
            run_planner_golden_case(&path, case);
        }
    }

    fn run_planner_golden_case(path: &std::path::Path, case: PlannerGoldenCase) {
        let mut observation = build_fixture_observation(&case);
        let policy = build_fixture_policy(&case.policy, observation.target_root_pid);
        let profiles = fixture_profiles(&case);
        let workload_policy = fixture_workload_policy(&case);
        let planner = CandidatePlanner::default_for_policy(&policy);

        let mut controller_state = ControllerRuntimeState::default();
        let state_candidate =
            state_candidate_for_action_kind(first_fixture_action_kind(&case), &profiles);

        if case.cooldown_active {
            controller_state.record_candidate_result(
                crate::autotune::controller::ControllerCandidateResultInput {
                    candidate: &state_candidate,
                    observation: &observation,
                    cpu_topology_signature: None,
                    result: CandidateMemoryResult::Reverted,
                    diagnostic_baseline_raw_score_total: Some(100),
                    diagnostic_current_raw_score_total: Some(120),
                    rollback_reason: Some("fixture cooldown".to_owned()),
                    cooldown_expires_unix_nanos: Some(observation.now_unix_nanos + 10_000),
                },
            );
        }

        if case.external_mutation {
            controller_state.active_experiment = Some(ActiveExperiment {
                experiment_id: ExperimentId::new("fixture-external-mutation"),
                candidate: state_candidate.clone(),
                baseline_score: window_score(100),
            });
            observation.active_config_snapshot =
                Some(active_nice_snapshot_for_tasks(&observation.active_tasks, 0));
        }

        if case.kept_conflict {
            observation.active_config_snapshot = None;
        }

        let active_profile_state = case.kept_conflict.then(|| {
            active_profile_state_with_kept(KeptCandidateState::new(
                ExperimentId::new("fixture-kept-conflict"),
                state_candidate.clone(),
                window_score(100),
                window_score(90),
                rollback_token(),
                observation.now_unix_nanos,
                "fixture kept conflict",
            ))
        });

        let mut dry_runner = CountingDryRunner::default();
        let result = planner.plan_with_dry_runner(
            PlannerInput {
                observation: &observation,
                daemon_policy: &policy,
                capabilities: &observation.capabilities,
                system_health: &observation.system_health,
                controller_state: &controller_state,
                active_profile_state: active_profile_state.as_ref(),
                workload_policy: &workload_policy,
                profiles: &profiles,
            },
            &mut dry_runner,
        );

        let selected_action_kind = result
            .selected
            .as_ref()
            .map(|candidate| candidate.action_kind().to_owned());
        assert_eq!(
            selected_action_kind,
            case.expected_selected_action_kind,
            "fixture {} selected action mismatch; evaluations={:#?}",
            path.display(),
            result.evaluations
        );

        if let Some(selected) = result.selected.as_ref() {
            assert!(
                !selected.is_high_risk_system_adjacent(),
                "fixture {} selected high-risk/system-adjacent candidate {}",
                path.display(),
                selected.candidate_name()
            );
        }

        assert_eq!(
            result.evaluations.len(),
            case.expected_total_proposals,
            "fixture {} total proposal mismatch; evaluations={:#?}",
            path.display(),
            result.evaluations
        );

        assert_eq!(
            result
                .evaluations
                .iter()
                .filter(|evaluation| evaluation.eligible)
                .count(),
            case.expected_eligible_proposals,
            "fixture {} eligible proposal mismatch; evaluations={:#?}",
            path.display(),
            result.evaluations
        );

        let actual_action_kinds = result
            .evaluations
            .iter()
            .map(|evaluation| evaluation.action_kind.clone())
            .collect::<Vec<_>>();
        let expected_action_kinds = case
            .expected_evaluations
            .iter()
            .map(|evaluation| evaluation.action_kind.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            actual_action_kinds,
            expected_action_kinds,
            "fixture {} action-kind list changed; evaluations={:#?}",
            path.display(),
            result.evaluations
        );

        for expected in &case.expected_evaluations {
            let evaluation = result
                .evaluations
                .iter()
                .find(|evaluation| evaluation.action_kind == expected.action_kind)
                .unwrap_or_else(|| {
                    panic!(
                        "fixture {} missing evaluation for {}",
                        path.display(),
                        expected.action_kind
                    )
                });

            let expected_objective =
                crate::autotune::workload_policy::parse_objective_kind(&expected.objective)
                    .unwrap_or_else(|err| {
                        panic!(
                            "fixture {} has invalid objective {}: {err}",
                            path.display(),
                            expected.objective
                        )
                    });
            assert_eq!(
                evaluation.objective,
                expected_objective,
                "fixture {} objective changed for {}",
                path.display(),
                expected.action_kind
            );

            assert_eq!(
                evaluation.eligible,
                expected.eligible,
                "fixture {} eligibility changed for {}; evaluation={:#?}",
                path.display(),
                expected.action_kind,
                evaluation
            );

            assert!(
                evaluation.confidence >= expected.min_confidence
                    && evaluation.confidence <= expected.max_confidence,
                "fixture {} confidence {:.3} outside expected range [{:.3}, {:.3}] for {}",
                path.display(),
                evaluation.confidence,
                expected.min_confidence,
                expected.max_confidence,
                expected.action_kind
            );

            let actual_dry_run_affected_tasks = evaluation
                .dry_run
                .as_ref()
                .map(|state| state.affected_tasks);
            assert_eq!(
                actual_dry_run_affected_tasks,
                expected.dry_run_affected_tasks,
                "fixture {} dry-run behavior changed for {}; evaluation={:#?}",
                path.display(),
                expected.action_kind,
                evaluation
            );

            assert_eq!(
                evaluation.candidate.manual_only_reason().is_some(),
                expected.manual_only,
                "fixture {} manual-only flag changed for {}",
                path.display(),
                expected.action_kind
            );

            let mut actual_reason_codes = evaluation
                .deny_reasons
                .iter()
                .map(CandidateDenyReason::reason_code)
                .map(str::to_owned)
                .collect::<Vec<_>>();
            actual_reason_codes.sort();

            let mut expected_reason_codes = expected.deny_reason_codes.clone();
            expected_reason_codes.sort();

            assert_eq!(
                actual_reason_codes,
                expected_reason_codes,
                "fixture {} deny reasons changed for {}; evaluation={:#?}",
                path.display(),
                expected.action_kind,
                evaluation
            );
        }

        let summary = result.summary();
        assert_eq!(summary.total_proposals, case.expected_total_proposals);
        assert_eq!(summary.eligible_proposals, case.expected_eligible_proposals);
    }

    fn build_fixture_policy(fixture: &DaemonPolicyFixture, tree_pid: Option<u32>) -> DaemonPolicy {
        let mode = fixture.mode.parse::<DaemonMode>().unwrap();
        let mut config = crate::autotune::runtime::daemon_config_for_runtime_mode(
            mode,
            ActionSource::AutotuneRuntime,
            tree_pid,
            None,
        );
        config.safety.allow_system_wide_suggestions = fixture.allow_system_wide_suggestions;
        config.autotune.allow_medium_risk_apply = fixture.allow_medium_risk_apply;
        config.safety.enabled_action_families =
            fixture.enabled_action_families.iter().cloned().collect();
        config.safety.system_wide_allowlist.irq_devices =
            fixture.irq_devices.iter().cloned().collect();
        config.safety.system_wide_allowlist.gpu_cards = fixture.gpu_cards.iter().cloned().collect();
        config.safety.system_wide_allowlist.vm_knobs = fixture.vm_knobs.iter().cloned().collect();
        config.autotune.allow_gpu_power_in_autotune = fixture.allow_gpu_power_in_autotune;
        config.autotune.allow_vm_knobs_in_autotune = fixture.allow_vm_knobs_in_autotune;

        if let Some(path) = &fixture.compile_cgroup {
            config.safety.cgroup_targets.compile_cgroup = Some(std::path::PathBuf::from(path));
        }
        if let Some(path) = &fixture.background_cgroup {
            config.safety.cgroup_targets.background_cgroup = Some(std::path::PathBuf::from(path));
        }
        if let Some(path) = &fixture.game_cgroup {
            config.safety.cgroup_targets.game_cgroup = Some(std::path::PathBuf::from(path));
        }

        build_daemon_policy(DaemonPolicyBuildInput {
            config: &config,
            remote_context: None,
        })
    }

    fn fixture_workload_policy(case: &PlannerGoldenCase) -> WorkloadPolicyMatrix {
        let mut policy = WorkloadPolicyMatrix::default_rules();
        if !case.policy.autonomous_families.is_empty() {
            let situation = parse_fixture_situation(&case.situation);
            if let Some(rule) = policy
                .rules
                .iter_mut()
                .find(|rule| rule.situation == situation)
            {
                rule.autonomous_families =
                    case.policy.autonomous_families.iter().cloned().collect();
            }
        }
        policy
    }

    fn build_fixture_observation(case: &PlannerGoldenCase) -> AutotuneObservation {
        let situation = parse_fixture_situation(&case.situation);
        let focus_kind = parse_fixture_focus_kind(&case.focus_kind);
        let mut observation = observation_for_situation(situation, focus_kind);

        observation.focus_confidence = 0.95;
        observation.situation.primary = situation;
        observation.situation.confidence = 0.95;
        if let Some(activity_level) = case.activity_level {
            observation.activity_level = activity_level;
        }
        observation.active_tasks = fixture_tasks(situation, focus_kind);
        observation.capabilities = DaemonCapabilities {
            btf_available: true,
            sched_tracepoints_available: true,
            perf_permissions_likely: true,
            cgroup_v2_available: true,
            sched_ext_available: true,
            uclamp_available: true,
            ionice_available: true,
            irq_affinity_available: true,
            gpu_sysfs_available: true,
            ..DaemonCapabilities::default()
        };
        observation.system_health = SystemHealthSnapshot {
            ok_for_apply: true,
            ..SystemHealthSnapshot::default()
        };

        if case.low_data_quality {
            observation.data_quality = OnlineDataQuality::Low {
                reasons: vec!["fixture low data quality".to_owned()],
            };
        }

        if case.critical_realtime {
            observation
                .focus_reasons
                .push("critical realtime input process present".to_owned());
        }

        if case.thermal_degraded {
            observation.objective_signals.thermal_degraded = Some(true);
            observation.objective_signals.thermal_throttle_count = Some(1);
            observation.objective_signals.signal_quality.thermal = ObjectiveSignalQuality::Direct;
        }

        let mut inventory = crate::system_inventory::SystemInventory {
            cpu_policies: Vec::new(),
            drm_devices: Vec::new(),
            irq_default_smp_affinity: Some("f".to_owned()),
            irq_lines: Vec::new(),
            power_source: crate::system_inventory::PowerSourceSnapshot {
                ac_online: Some(true),
                battery_present: false,
                battery_discharging: None,
            },
            sched_ext_available: true,
            vm_knobs: std::collections::BTreeMap::new(),
            inventory_hash: format!("fixture-{}", case.situation),
        };
        let mut active_config = ActiveConfigSnapshot::default();
        seed_task_active_config(&mut active_config, &observation.active_tasks);

        if case.cpu_power_evidence {
            observation.objective_signals.cpu_power_limited = Some(true);
            observation.objective_signals.cpu_power_limited_cpu = Some(0);
            observation.objective_signals.signal_quality.cpu_power =
                ObjectiveSignalQuality::Derived;
            observation.objective_signals.thermal_degraded = Some(false);
            observation.objective_signals.thermal_throttle_count = Some(0);
            observation.objective_signals.signal_quality.thermal = ObjectiveSignalQuality::Direct;
            inventory
                .cpu_policies
                .push(crate::system_inventory::CpuPolicyInventory {
                    policy: "policy0".to_owned(),
                    path: std::path::PathBuf::from("/fake/sys/devices/system/cpu/cpufreq/policy0"),
                    scaling_governor: Some("powersave".to_owned()),
                    available_governors: Some("powersave performance".to_owned()),
                    energy_performance_preference: Some("balance_power".to_owned()),
                    energy_performance_available_preferences: Some(
                        "balance_power performance".to_owned(),
                    ),
                    related_cpus: Some("0 1".to_owned()),
                });
            active_config.cpu_power.policies.push(
                crate::autotune::observation::CpuPolicyRuntimeState {
                    policy: "policy0".to_owned(),
                    scaling_governor: Some("powersave".to_owned()),
                    energy_performance_preference: Some("balance_power".to_owned()),
                    related_cpus: Some("0 1".to_owned()),
                },
            );
        }

        if case.gpu_power_evidence {
            observation.objective_signals.gpu_power_limited = Some(true);
            observation.objective_signals.gpu_busy_percent = Some(96);
            observation.objective_signals.gpu_clock_mhz = Some(250);
            observation.objective_signals.gpu_temp_millidegrees = Some(70_000);
            observation.objective_signals.gpu_active_render_node = Some("renderD128".to_owned());
            observation.objective_signals.signal_quality.gpu_power = ObjectiveSignalQuality::Direct;
            observation
                .objective_signals
                .signal_quality
                .gpu_active_render_node = ObjectiveSignalQuality::Direct;
            inventory
                .drm_devices
                .push(crate::system_inventory::DrmDeviceInventory {
                    name: "card0".to_owned(),
                    path: std::path::PathBuf::from("/fake/sys/class/drm/card0"),
                    render_node: Some("renderD128".to_owned()),
                    pci_id: Some("1002:744c".to_owned()),
                    vendor: Some("amd".to_owned()),
                    hwmon_paths: Vec::new(),
                });
            active_config.gpu_power.devices.push(
                crate::autotune::observation::GpuPowerRuntimeState {
                    device: "card0".to_owned(),
                    power_dpm_force_performance_level: Some("auto".to_owned()),
                    pp_power_profile_mode: Some("BOOTUP_DEFAULT".to_owned()),
                },
            );
        }

        if case.irq_evidence {
            observation.objective_signals.irq_overlap_count = Some(2);
            observation.objective_signals.irq_worst_overlap_ns = Some(4_000_000);
            observation.objective_signals.irq_hot_irq = Some(146);
            observation.objective_signals.irq_hot_cpu = Some(2);
            observation.objective_signals.signal_quality.irq_overlap =
                ObjectiveSignalQuality::Direct;
            active_config.irq.per_irq.insert(146, "4".to_owned());
            inventory.irq_lines.push(crate::irq_inspect::IrqLine {
                irq: "146".to_owned(),
                counts_by_cpu: vec![10_000, 2, 20_000, 30_000],
                total: 60_002,
                kind: "PCI-MSI".to_owned(),
                name: "amdgpu".to_owned(),
                raw: "146: 10000 2 20000 30000 PCI-MSI amdgpu".to_owned(),
            });
        }

        if case.vm_evidence {
            observation.objective_signals.swap_activity_events = Some(3);
            observation.objective_signals.signal_quality.swap_activity =
                ObjectiveSignalQuality::Approximate;
            observation.objective_signals.dirty_writeback_events = Some(5);
            observation.objective_signals.signal_quality.dirty_writeback =
                ObjectiveSignalQuality::Direct;
            observation
                .objective_signals
                .memory_pressure_some_avg10_percent = Some(12.5);
            observation.objective_signals.signal_quality.memory_pressure =
                ObjectiveSignalQuality::Direct;
            inventory
                .vm_knobs
                .insert("proc/sys/vm/swappiness".to_owned(), "60".to_owned());
            active_config
                .vm
                .knobs
                .insert("proc/sys/vm/swappiness".to_owned(), "60".to_owned());
        }

        if let Some(value) = case.hardware_signals.clone() {
            let mut signals: ObjectiveSignals = serde_json::from_value(value)
                .unwrap_or_else(|err| panic!("invalid hardware_signals fixture: {err}"));
            if signals.irq_overlap_count.is_some() || signals.irq_worst_overlap_ns.is_some() {
                signals.signal_quality.irq_overlap = ObjectiveSignalQuality::Direct;
            }
            if signals.block_io_overlap_count.is_some()
                || signals.block_io_worst_latency_ns.is_some()
            {
                signals.signal_quality.block_io_overlap = ObjectiveSignalQuality::Direct;
            }
            if signals.thermal_degraded.is_some() || signals.thermal_throttle_count.is_some() {
                signals.signal_quality.thermal = ObjectiveSignalQuality::Direct;
            }
            if signals.cpu_power_limited.is_some() || signals.cpu_power_limited_cpu.is_some() {
                signals.signal_quality.cpu_power = ObjectiveSignalQuality::Derived;
            }
            if signals.gpu_power_limited.is_some()
                || signals.gpu_busy_percent.is_some()
                || signals.gpu_clock_mhz.is_some()
                || signals.gpu_temp_millidegrees.is_some()
            {
                signals.signal_quality.gpu_power = ObjectiveSignalQuality::Direct;
            }
            if signals.gpu_active_render_node.is_some() {
                signals.signal_quality.gpu_active_render_node = ObjectiveSignalQuality::Direct;
            }
            if signals.memory_pressure_some_avg10_percent.is_some() {
                signals.signal_quality.memory_pressure = ObjectiveSignalQuality::Direct;
            }
            if signals.swap_activity_events.is_some() {
                signals.signal_quality.swap_activity = ObjectiveSignalQuality::Approximate;
            }
            if signals.dirty_writeback_events.is_some() {
                signals.signal_quality.dirty_writeback = ObjectiveSignalQuality::Direct;
            }

            if let Some(irq) = signals.irq_hot_irq {
                active_config.irq.per_irq.insert(irq, "4".to_owned());
                inventory.irq_lines.push(crate::irq_inspect::IrqLine {
                    irq: irq.to_string(),
                    counts_by_cpu: vec![10_000, 2, 20_000, 30_000],
                    total: 60_002,
                    kind: "PCI-MSI".to_owned(),
                    name: "amdgpu".to_owned(),
                    raw: format!("{irq}: 10000 2 20000 30000 PCI-MSI amdgpu"),
                });
            }

            if signals.gpu_power_limited == Some(true)
                || signals.gpu_busy_percent.is_some()
                || signals.gpu_active_render_node.is_some()
            {
                let render_node = signals
                    .gpu_active_render_node
                    .clone()
                    .unwrap_or_else(|| "renderD128".to_owned());
                inventory
                    .drm_devices
                    .push(crate::system_inventory::DrmDeviceInventory {
                        name: "card0".to_owned(),
                        path: std::path::PathBuf::from("/fake/sys/class/drm/card0"),
                        render_node: Some(render_node),
                        pci_id: Some("1002:744c".to_owned()),
                        vendor: Some("amd".to_owned()),
                        hwmon_paths: Vec::new(),
                    });
                active_config.gpu_power.devices.push(
                    crate::autotune::observation::GpuPowerRuntimeState {
                        device: "card0".to_owned(),
                        power_dpm_force_performance_level: Some("auto".to_owned()),
                        pp_power_profile_mode: Some("BOOTUP_DEFAULT".to_owned()),
                    },
                );
            }

            if signals
                .memory_pressure_some_avg10_percent
                .is_some_and(|value| value > 0.0)
                || signals.swap_activity_events.is_some_and(|value| value > 0)
                || signals
                    .dirty_writeback_events
                    .is_some_and(|value| value > 0)
            {
                inventory
                    .vm_knobs
                    .insert("proc/sys/vm/swappiness".to_owned(), "60".to_owned());
                active_config
                    .vm
                    .knobs
                    .insert("proc/sys/vm/swappiness".to_owned(), "60".to_owned());
            }

            observation.objective_signals = signals;
        }

        let system_context = SystemContextSnapshot {
            capabilities: observation.capabilities.clone(),
            health: observation.system_health.clone(),
            inventory,
            active_config: active_config.clone(),
            sampled_at_unix_nanos: observation.now_unix_nanos,
        };

        observation.active_config_snapshot = Some(active_config);
        observation.system_context = Some(system_context);
        observation
    }

    fn seed_task_active_config(
        active_config: &mut ActiveConfigSnapshot,
        active_tasks: &[ActiveTaskSnapshot],
    ) {
        for task in active_tasks {
            active_config
                .affinity
                .per_tid
                .entry(task.tid.as_u32())
                .or_insert_with(|| "0-3".to_owned());
            active_config
                .nice
                .per_tid
                .entry(task.tid.as_u32())
                .or_insert(0);
            active_config
                .ionice
                .per_tid
                .entry(task.tid.as_u32())
                .or_insert_with(|| "best-effort:4".to_owned());
            active_config
                .uclamp
                .per_tid
                .entry(task.tid.as_u32())
                .or_insert(UclampValues {
                    sched_util_min: Some(0),
                    sched_util_max: Some(1024),
                });
            if let Some(cgroup_path) = &task.cgroup_path {
                active_config
                    .cgroup
                    .per_tid
                    .entry(task.tid.as_u32())
                    .or_insert_with(|| cgroup_path.clone());
            }
        }
    }

    fn active_nice_snapshot_for_tasks(
        active_tasks: &[ActiveTaskSnapshot],
        nice: i32,
    ) -> ActiveConfigSnapshot {
        let mut snapshot = ActiveConfigSnapshot::default();
        for task in active_tasks {
            snapshot.nice.per_tid.insert(task.tid.as_u32(), nice);
        }
        snapshot
    }

    fn fixture_tasks(
        situation: SituationKind,
        focus_kind: FocusGroupKind,
    ) -> Vec<ActiveTaskSnapshot> {
        match (situation, focus_kind) {
            (
                SituationKind::GameFocused
                | SituationKind::GameCpuSchedulerPressure
                | SituationKind::GameGpuBound,
                _,
            ) => vec![
                fixture_task(1234, "game-main", TaskClass::Game),
                fixture_task(1235, "game-render", TaskClass::GameRenderThread),
                fixture_task(1236, "game-worker", TaskClass::GameWorkerThread),
            ],
            (_, FocusGroupKind::Browser) => vec![
                fixture_task(1234, "browser-main", TaskClass::BrowserForeground),
                fixture_task(1235, "browser-renderer", TaskClass::BrowserRenderer),
                fixture_task(1236, "browser-gpu", TaskClass::BrowserGpu),
            ],
            (_, FocusGroupKind::Compile) => vec![
                fixture_task(1234, "rustc", TaskClass::Compiler),
                fixture_task(1235, "ld.lld", TaskClass::Linker),
            ],
            (SituationKind::CompositorPressure, _) => {
                vec![fixture_task(1234, "compositor-helper", TaskClass::Helper)]
            }
            (_, FocusGroupKind::Media) => {
                vec![fixture_task(1234, "media-player", TaskClass::Media)]
            }
            (_, FocusGroupKind::Recording) => {
                vec![fixture_task(1234, "recorder", TaskClass::Recorder)]
            }
            (_, FocusGroupKind::VirtualMachine) => {
                vec![fixture_task(1234, "qemu", TaskClass::VirtualMachine)]
            }
            _ => vec![fixture_task(1234, "desktop-helper", TaskClass::Helper)],
        }
    }

    fn fixture_task(tid: u32, comm: &str, class: TaskClass) -> ActiveTaskSnapshot {
        let process_starttime_ticks = Some(10);
        let task_starttime_ticks = if tid == 1234 {
            process_starttime_ticks
        } else {
            Some(u64::from(tid))
        };

        ActiveTaskSnapshot {
            tid: tid.into(),
            process_pid: (1234).into(),
            comm: comm.to_owned(),
            class,
            process_starttime_ticks,
            task_starttime_ticks,
            cgroup_path: Some("/user.slice/fixture.scope".to_owned()),
        }
    }

    fn fixture_profiles(case: &PlannerGoldenCase) -> Vec<Profile> {
        if !case
            .policy
            .enabled_action_families
            .iter()
            .any(|family| family == "cpu_affinity_profile")
        {
            return Vec::new();
        }

        let profile_name =
            if parse_fixture_situation(&case.situation) == SituationKind::CompositorPressure {
                "fixture-game-compositor-separate"
            } else {
                "fixture-game-helper"
            };

        vec![Profile {
            name: profile_name.to_owned(),
            rules: vec![ProfileRule {
                affinity: Some(CpuMask::parse("0").unwrap()),
                nice: None,
                ionice: None,
                match_class: vec![
                    TaskClass::Game,
                    TaskClass::GameHelper,
                    TaskClass::GameRenderThread,
                    TaskClass::GameWorkerThread,
                    TaskClass::Helper,
                ],
                match_comm: Vec::new(),
            }],
        }]
    }

    fn first_fixture_action_kind(case: &PlannerGoldenCase) -> &str {
        case.expected_evaluations
            .first()
            .map(|evaluation| evaluation.action_kind.as_str())
            .or_else(|| {
                case.policy
                    .enabled_action_families
                    .first()
                    .map(String::as_str)
            })
            .unwrap_or("cpu_affinity_profile")
    }

    fn state_candidate_for_action_kind(action_kind: &str, profiles: &[Profile]) -> CandidateAction {
        if action_kind == "cpu_affinity_profile" {
            let profile = profiles.first().cloned().unwrap_or_else(|| {
                if let CandidateAction::CpuAffinityProfile { plan } =
                    cpu_affinity_candidate("fixture-game-helper")
                {
                    plan.profile
                } else {
                    panic!("expected cpu affinity candidate")
                }
            });
            return CandidateAction::cpu_affinity_profile(profile, 1234);
        }

        candidate_for_action_kind(action_kind)
    }

    fn parse_fixture_situation(value: &str) -> SituationKind {
        crate::autotune::workload_policy::parse_situation_kind(value).unwrap()
    }

    fn parse_fixture_focus_kind(value: &str) -> FocusGroupKind {
        match value {
            "Game" => FocusGroupKind::Game,
            "Desktop" => FocusGroupKind::Desktop,
            "Browser" => FocusGroupKind::Browser,
            "Compile" => FocusGroupKind::Compile,
            "Recording" => FocusGroupKind::Recording,
            "Media" => FocusGroupKind::Media,
            "VirtualMachine" => FocusGroupKind::VirtualMachine,
            other => panic!("unsupported focus kind {other}"),
        }
    }
}
