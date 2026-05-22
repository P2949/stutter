//! Policy denial planner tests extracted from `autotune::planner`.
//!
//! Owns provider confidence, manual-only, policy, capability, observe-mode, and allowlist denial tests.
//! Does not own shared fixtures or production planner behavior.

#[cfg(test)]
mod tests {
    use super::super::{
        super::{CandidateDenyReason, CandidatePlanner, PlannerInput},
        support::*,
    };

    #[test]
    fn low_confidence_suggest_candidate_is_denied_by_provider_threshold() {
        let evaluation = evaluate_static_candidate_with_confidence(
            DaemonMode::Suggest,
            SituationKind::GameCpuSchedulerPressure,
            FocusGroupKind::Game,
            cpu_affinity_candidate("low-confidence-suggest"),
            0.49,
        );

        assert_eq!(evaluation.confidence, 0.49);
        assert_eq!(evaluation.descriptor.confidence, Some(0.49));
        assert!(
            evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::ProviderConfidenceTooLow)
        );
        assert!(
            !evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::WorkloadPolicyBlocked)
        );
        assert!(
            !evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::ObjectiveNotAllowedForWorkload)
        );
    }

    #[test]
    fn low_confidence_high_risk_suggestion_is_denied_even_when_workload_allows_it() {
        let evaluation = evaluate_static_candidate_with_confidence(
            DaemonMode::Suggest,
            SituationKind::GameCpuSchedulerPressure,
            FocusGroupKind::Game,
            cpu_power_candidate("low-confidence-cpu-power"),
            0.89,
        );

        assert_eq!(evaluation.confidence, 0.89);
        assert_eq!(evaluation.descriptor.confidence, Some(0.89));
        assert!(
            evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::ProviderConfidenceTooLow)
        );
        assert!(
            !evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::WorkloadPolicyBlocked)
        );
        assert!(
            !evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::ObjectiveNotAllowedForWorkload)
        );
    }

    #[test]
    fn suggest_manual_only_high_risk_candidate_is_not_eligible_and_does_not_dry_run() {
        let candidate = high_risk_irq_affinity_candidate("suggest-irq-manual-only");
        let policy = policy(DaemonMode::Suggest);
        let mut observation =
            observation_for_situation(SituationKind::IrqPressure, FocusGroupKind::Game);
        enable_capability_for_candidate(&mut observation, &candidate);
        let mut dry_runner = CountingDryRunner::default();

        let evaluation = evaluate_candidate_with_runner(
            &policy,
            &observation,
            &observation.capabilities,
            &ControllerRuntimeState::default(),
            candidate,
            1.0,
            &mut dry_runner,
        );

        assert_eq!(dry_runner.calls, 0);
        assert!(evaluation.dry_run.is_none());
        assert!(!evaluation.eligible);
        assert!(
            evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::ManualOnlyHighRisk)
        );
        assert!(
            evaluation
                .deny_messages
                .iter()
                .any(|message| message.contains("manual-only high-risk/system-adjacent"))
        );
    }

    #[test]
    fn explicit_high_risk_dry_run_collects_diagnostics_without_eligibility() {
        let candidate = high_risk_irq_affinity_candidate("suggest-irq-dry-run");
        let mut config = crate::autotune::runtime::daemon_config_for_runtime_mode(
            DaemonMode::Suggest,
            ActionSource::AutotuneRuntime,
            Some(1234),
            None,
        );
        config.safety.allow_system_wide_suggestions = true;
        config
            .safety
            .system_wide_allowlist
            .irq_devices
            .insert("test-device".to_owned());
        config.autotune.high_risk_dry_run = true;
        let policy = build_daemon_policy(DaemonPolicyBuildInput {
            config: &config,
            remote_context: None,
        });
        let mut observation =
            observation_for_situation(SituationKind::IrqPressure, FocusGroupKind::Game);
        enable_capability_for_candidate(&mut observation, &candidate);
        let mut dry_runner = CountingDryRunner::default();

        let evaluation = evaluate_candidate_with_runner(
            &policy,
            &observation,
            &observation.capabilities,
            &ControllerRuntimeState::default(),
            candidate,
            1.0,
            &mut dry_runner,
        );

        assert_eq!(dry_runner.calls, 1);
        assert!(evaluation.dry_run.is_some());
        assert!(!evaluation.eligible);
        assert!(
            evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::ManualOnlyHighRisk)
        );
    }

    #[test]
    fn low_confidence_medium_risk_candidate_is_denied_even_when_family_and_objective_allowed() {
        let evaluation = evaluate_static_candidate_with_confidence(
            DaemonMode::ApplyMediumRisk,
            SituationKind::GameCpuSchedulerPressure,
            FocusGroupKind::Game,
            uclamp_candidate("low-confidence-uclamp"),
            0.84,
        );

        assert_eq!(evaluation.confidence, 0.84);
        assert_eq!(evaluation.descriptor.confidence, Some(0.84));
        assert!(
            evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::ProviderConfidenceTooLow)
        );
        assert!(
            !evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::WorkloadPolicyBlocked)
        );
        assert!(
            !evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::ObjectiveNotAllowedForWorkload)
        );
    }

    #[test]
    fn policy_denied_candidate_does_not_call_dry_run() {
        let candidate = cpu_affinity_candidate("policy-denied-no-dry-run");
        let mut policy = policy(DaemonMode::Suggest);
        policy
            .denied_action_families
            .insert("cpu_affinity_profile".to_owned());
        let observation = observation();
        let mut dry_runner = CountingDryRunner::default();

        let evaluation = evaluate_candidate_with_runner(
            &policy,
            &observation,
            &observation.capabilities,
            &ControllerRuntimeState::default(),
            candidate,
            1.0,
            &mut dry_runner,
        );

        assert_eq!(dry_runner.calls, 0);
        assert!(evaluation.dry_run.is_none());
        assert!(
            evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::DeniedFamily)
        );
    }

    #[test]
    fn low_confidence_candidate_does_not_call_dry_run() {
        let candidate = cpu_affinity_candidate("low-confidence-no-dry-run");
        let policy = policy(DaemonMode::Suggest);
        let observation = observation();
        let mut dry_runner = CountingDryRunner::default();

        let evaluation = evaluate_candidate_with_runner(
            &policy,
            &observation,
            &observation.capabilities,
            &ControllerRuntimeState::default(),
            candidate,
            0.49,
            &mut dry_runner,
        );

        assert_eq!(dry_runner.calls, 0);
        assert!(evaluation.dry_run.is_none());
        assert!(
            evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::ProviderConfidenceTooLow)
        );
    }

    #[test]
    fn non_autonomous_apply_candidate_does_not_call_dry_run() {
        let candidate = uclamp_candidate("non-autonomous-no-dry-run");
        let policy = policy(DaemonMode::ApplyMediumRisk);
        let mut observation = observation_for_situation(
            SituationKind::GameCpuSchedulerPressure,
            FocusGroupKind::Game,
        );
        enable_capability_for_candidate(&mut observation, &candidate);
        let mut dry_runner = CountingDryRunner::default();

        let evaluation = evaluate_candidate_with_runner(
            &policy,
            &observation,
            &observation.capabilities,
            &ControllerRuntimeState::default(),
            candidate,
            1.0,
            &mut dry_runner,
        );

        assert_eq!(dry_runner.calls, 0);
        assert!(evaluation.dry_run.is_none());
        assert!(
            evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::NotAutonomousForWorkload)
        );
    }

    #[test]
    fn cooldown_blocked_candidate_does_not_call_dry_run() {
        let candidate = cpu_affinity_candidate("cooldown-no-dry-run");
        let policy = policy(DaemonMode::Suggest);
        let observation = observation();
        let mut controller_state = ControllerRuntimeState::default();
        controller_state.record_candidate_result(
            crate::autotune::controller::ControllerCandidateResultInput {
                candidate: &candidate,
                observation: &observation,
                cpu_topology_signature: None,
                result: CandidateMemoryResult::Tried,
                diagnostic_baseline_raw_score_total: None,
                diagnostic_current_raw_score_total: None,
                rollback_reason: None,
                cooldown_expires_unix_nanos: Some(observation.now_unix_nanos + 1_000_000_000),
            },
        );
        let mut dry_runner = CountingDryRunner::default();

        let evaluation = evaluate_candidate_with_runner(
            &policy,
            &observation,
            &observation.capabilities,
            &controller_state,
            candidate,
            1.0,
            &mut dry_runner,
        );

        assert_eq!(dry_runner.calls, 0);
        assert!(evaluation.dry_run.is_none());
        assert!(
            evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::CooldownActive)
        );
    }

    #[test]
    fn cgroup_not_allowlisted_candidate_does_not_call_dry_run() {
        let candidate = cgroup_candidate("/user.slice/not-allowlisted.slice");
        let policy = policy(DaemonMode::Suggest);
        let mut observation =
            observation_for_situation(SituationKind::CompileCpuBound, FocusGroupKind::Compile);
        enable_capability_for_candidate(&mut observation, &candidate);
        let mut capabilities = observation.capabilities.clone();
        capabilities.cgroup_v2_available = true;
        let mut dry_runner = CountingDryRunner::default();

        let evaluation = evaluate_candidate_with_runner(
            &policy,
            &observation,
            &capabilities,
            &ControllerRuntimeState::default(),
            candidate,
            1.0,
            &mut dry_runner,
        );

        assert_eq!(dry_runner.calls, 0);
        assert!(evaluation.dry_run.is_none());
        assert!(
            evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::CgroupTargetNotAllowlisted)
        );
    }

    #[test]
    fn system_wide_allowlist_denies_cpu_power_policy_outside_allowlist() {
        let mut candidate = cpu_power_candidate("cpu-power-policy9");
        let CandidateAction::CpuPower { plan } = &mut candidate else {
            panic!("expected CPU power candidate");
        };
        plan.evidence.push(CandidateEvidence::new(
            "cpu_power_structured",
            "policy=policy9 related_cpus=[0]",
            1.0,
        ));

        let mut policy = policy(DaemonMode::Suggest);
        policy
            .system_wide_allowlist
            .cpu_policies
            .insert("policy0".to_owned());
        let mut observation = observation_for_situation(
            SituationKind::GameCpuSchedulerPressure,
            FocusGroupKind::Game,
        );
        enable_capability_for_candidate(&mut observation, &candidate);
        let mut dry_runner = CountingDryRunner::default();

        let evaluation = evaluate_candidate_with_runner(
            &policy,
            &observation,
            &observation.capabilities,
            &ControllerRuntimeState::default(),
            candidate,
            1.0,
            &mut dry_runner,
        );

        assert_eq!(dry_runner.calls, 0);
        assert!(!evaluation.eligible);
        assert!(
            evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::SystemWideTargetNotAllowlisted)
        );
        assert!(
            evaluation
                .deny_messages
                .iter()
                .any(|message| message.contains("policy9"))
        );
    }

    #[test]
    fn system_wide_allowlist_denies_gpu_card_outside_allowlist() {
        let candidate = gpu_power_candidate("gpu-card-denied");
        let mut policy = policy(DaemonMode::Suggest);
        policy
            .system_wide_allowlist
            .gpu_cards
            .insert("card1".to_owned());
        let mut observation =
            observation_for_situation(SituationKind::GameGpuBound, FocusGroupKind::Game);
        enable_capability_for_candidate(&mut observation, &candidate);
        let mut dry_runner = CountingDryRunner::default();

        let evaluation = evaluate_candidate_with_runner(
            &policy,
            &observation,
            &observation.capabilities,
            &ControllerRuntimeState::default(),
            candidate,
            1.0,
            &mut dry_runner,
        );

        assert_eq!(dry_runner.calls, 0);
        assert!(!evaluation.eligible);
        assert!(
            evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::SystemWideTargetNotAllowlisted)
        );
        assert!(
            evaluation
                .deny_messages
                .iter()
                .any(|message| message.contains("card0"))
        );
    }

    #[test]
    fn system_wide_allowlist_denies_irq_device_outside_allowlist() {
        let candidate = irq_affinity_candidate("irq-device-denied");
        let mut policy = policy(DaemonMode::Suggest);
        policy
            .system_wide_allowlist
            .irq_devices
            .insert("amdgpu".to_owned());
        let mut observation =
            observation_for_situation(SituationKind::IrqPressure, FocusGroupKind::Desktop);
        enable_capability_for_candidate(&mut observation, &candidate);
        let mut dry_runner = CountingDryRunner::default();

        let evaluation = evaluate_candidate_with_runner(
            &policy,
            &observation,
            &observation.capabilities,
            &ControllerRuntimeState::default(),
            candidate,
            1.0,
            &mut dry_runner,
        );

        assert_eq!(dry_runner.calls, 0);
        assert!(!evaluation.eligible);
        assert!(
            evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::SystemWideTargetNotAllowlisted)
        );
        assert!(
            evaluation
                .deny_messages
                .iter()
                .any(|message| message.contains("test-device"))
        );
    }

    #[test]
    fn system_wide_allowlist_denies_vm_knob_outside_allowlist() {
        let candidate = vm_knob_candidate("vm-knob-denied");
        let mut policy = policy(DaemonMode::Suggest);
        policy
            .system_wide_allowlist
            .vm_knobs
            .insert("proc/sys/vm/dirty_ratio".to_owned());
        let mut observation =
            observation_for_situation(SituationKind::IoPressure, FocusGroupKind::Desktop);
        enable_capability_for_candidate(&mut observation, &candidate);
        let mut dry_runner = CountingDryRunner::default();

        let evaluation = evaluate_candidate_with_runner(
            &policy,
            &observation,
            &observation.capabilities,
            &ControllerRuntimeState::default(),
            candidate,
            1.0,
            &mut dry_runner,
        );

        assert_eq!(dry_runner.calls, 0);
        assert!(!evaluation.eligible);
        assert!(
            evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::SystemWideTargetNotAllowlisted)
        );
        assert!(
            evaluation
                .deny_messages
                .iter()
                .any(|message| message.contains("vm/swappiness"))
        );
    }

    #[test]
    fn system_wide_allowlist_allows_explicit_targets_to_reach_manual_only_gate() {
        let candidate = gpu_power_candidate("gpu-card-allowlisted");
        let mut config = crate::autotune::runtime::daemon_config_for_runtime_mode(
            DaemonMode::Suggest,
            ActionSource::AutotuneRuntime,
            Some(1234),
            None,
        );
        config.safety.allow_system_wide_suggestions = true;
        config
            .safety
            .system_wide_allowlist
            .gpu_cards
            .insert("card0".to_owned());
        config.autotune.high_risk_dry_run = true;
        let policy = build_daemon_policy(DaemonPolicyBuildInput {
            config: &config,
            remote_context: None,
        });
        let mut observation =
            observation_for_situation(SituationKind::GameGpuBound, FocusGroupKind::Game);
        enable_capability_for_candidate(&mut observation, &candidate);
        let mut dry_runner = CountingDryRunner::default();

        let evaluation = evaluate_candidate_with_runner(
            &policy,
            &observation,
            &observation.capabilities,
            &ControllerRuntimeState::default(),
            candidate,
            1.0,
            &mut dry_runner,
        );

        assert_eq!(dry_runner.calls, 1);
        assert!(evaluation.dry_run.is_some());
        assert!(!evaluation.eligible);
        assert!(
            evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::ManualOnlyHighRisk)
        );
        assert!(
            !evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::SystemWideTargetNotAllowlisted)
        );
    }

    #[test]
    fn observe_mode_returns_no_suggestions() {
        let policy = policy(DaemonMode::Observe);
        let planner = CandidatePlanner::default_for_policy(&policy);
        let observation = observation();

        let result = planner.plan(PlannerInput {
            observation: &observation,
            daemon_policy: &policy,
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            controller_state: &ControllerRuntimeState::default(),
            active_profile_state: None,
            workload_policy: &WorkloadPolicyMatrix::default_rules(),
            profiles: &[],
        });

        assert!(result.selected.is_none());
        assert!(result.evaluations.is_empty());
    }

    #[test]
    fn low_risk_policy_denies_medium_risk_fake_candidate() {
        let policy = policy(DaemonMode::ApplyLowRisk);
        let candidate = CandidateAction::fake(
            crate::actions::ActionId::new("fake-medium".to_owned()),
            SafetyClass::ReversibleMediumRisk,
        );

        assert!(candidate.safety_class() > policy.max_safety_class);
    }

    #[test]
    fn cgroup_provider_candidate_is_denied_when_cgroup_v2_capability_missing() {
        let policy = suggest_policy_with_compile_cgroup();
        let planner = CandidatePlanner::default_for_policy(&policy);
        let mut observation = observation();
        observation.primary_situation = SituationKind::CompileCpuBound;
        observation.focus_kind = Some(FocusGroupKind::Compile);
        observation.active_tasks = vec![ActiveTaskSnapshot {
            tid: 1234,
            process_pid: 1234,
            comm: "rustc".to_owned(),
            class: TaskClass::Compiler,
            process_starttime_ticks: Some(10),
            task_starttime_ticks: Some(10),
            cgroup_path: Some("/user.slice/app.scope".to_owned()),
        }];
        observation.refresh_situation_classification();

        let result = planner.plan(PlannerInput {
            observation: &observation,
            daemon_policy: &policy,
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            controller_state: &ControllerRuntimeState::default(),
            active_profile_state: None,
            workload_policy: &WorkloadPolicyMatrix::default_rules(),
            profiles: &[],
        });

        let cgroup = result
            .evaluations
            .iter()
            .find(|evaluation| evaluation.action_kind == "cgroup_placement")
            .expect("expected cgroup evaluation");
        assert!(
            cgroup
                .deny_reasons
                .contains(&CandidateDenyReason::CapabilityMissing)
        );
    }

    #[test]
    fn planner_denies_cgroup_candidate_outside_allowlist() {
        let policy = suggest_policy_with_compile_cgroup();
        let mut registry = CandidateProviderRegistry::default();
        registry.register(Box::new(StaticProvider {
            candidate: cgroup_candidate("/user.slice/not-allowlisted.slice"),
        }));
        let planner = CandidatePlanner::new(registry);
        let observation = observation();

        let result = planner.plan(PlannerInput {
            observation: &observation,
            daemon_policy: &policy,
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            controller_state: &ControllerRuntimeState::default(),
            active_profile_state: None,
            workload_policy: &WorkloadPolicyMatrix::default_rules(),
            profiles: &[],
        });

        assert!(
            result.evaluations[0]
                .deny_reasons
                .contains(&CandidateDenyReason::CgroupTargetNotAllowlisted)
        );
    }
}
