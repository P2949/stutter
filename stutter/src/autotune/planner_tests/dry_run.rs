//! Dry-run planner tests extracted from `autotune::planner`.
//!
//! Owns dry-run skipping, no-effective-change, conflict, kept-action, and external-mutation tests.
//! Does not own shared fixtures or production planner behavior.

#[cfg(test)]
mod tests {
    use super::super::{
        super::{CandidateDenyReason, CandidatePlanner, PlannerInput},
        support::*,
    };

    #[test]
    fn no_effective_change_candidate_is_denied_before_dry_run() {
        let candidate = nice_candidate();
        let policy = policy(DaemonMode::Suggest);
        let observation = observation_with_active_nice(
            SituationKind::CompileCpuBound,
            FocusGroupKind::Compile,
            1234,
            5,
        );
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
                .contains(&CandidateDenyReason::NoEffectiveChange)
        );
        assert!(
            evaluation
                .deny_messages
                .iter()
                .any(|message| message.contains("would not change active configuration"))
        );
    }

    #[test]
    fn cpu_affinity_no_effective_change_is_denied_before_dry_run() {
        let candidate = cpu_affinity_candidate("cpu-affinity-noop");
        let policy = policy(DaemonMode::Suggest);
        let observation = observation_with_cpu_affinity("0");
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
        assert!(
            evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::NoEffectiveChange)
        );
    }

    #[test]
    fn cpu_affinity_external_mutation_is_detected_before_dry_run() {
        let active_candidate = cpu_affinity_candidate("active-cpu-affinity");
        let candidate = cpu_affinity_candidate("new-cpu-affinity");
        let policy = policy(DaemonMode::Suggest);
        let observation = observation_with_cpu_affinity("1");
        let controller_state = ControllerRuntimeState {
            active_experiment: Some(ActiveExperiment {
                experiment_id: ExperimentId::new("external-cpu-affinity"),
                candidate: active_candidate,
                baseline_score_total: 1_000,
            }),
            ..ControllerRuntimeState::default()
        };
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
        assert!(
            evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::ExternalMutationDetected)
        );
    }

    #[test]
    fn cpu_affinity_kept_action_drift_is_detected_before_dry_run() {
        let kept_candidate = cpu_affinity_candidate("kept-cpu-affinity");
        let candidate = nice_candidate();
        let policy = policy(DaemonMode::Suggest);
        let observation = observation_with_cpu_affinity("1");
        let active_profile_state = active_profile_state_with_kept(KeptCandidateState::new(
            ExperimentId::new("kept-cpu-affinity-drift"),
            kept_candidate,
            window_score(1_000),
            window_score(800),
            rollback_token(),
            observation.now_unix_nanos,
            "kept for CPU affinity drift test",
        ));
        let mut dry_runner = CountingDryRunner::default();
        let proposals = vec![proposal_for_candidate(candidate, 1.0)];

        let mut evaluations = super::super::super::evaluate_proposals_with_runner(
            PlannerInput {
                observation: &observation,
                daemon_policy: &policy,
                capabilities: &observation.capabilities,
                system_health: &observation.system_health,
                controller_state: &ControllerRuntimeState::default(),
                active_profile_state: Some(&active_profile_state),
                workload_policy: &WorkloadPolicyMatrix::default_rules(),
                profiles: &[],
            },
            proposals,
            &mut dry_runner,
        );

        let evaluation = evaluations.remove(0);
        assert_eq!(dry_runner.calls, 0);
        assert!(
            evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::KeptActionNoLongerActive)
        );
    }

    #[test]
    fn active_experiment_external_mutation_blocks_conflicting_candidate_before_dry_run() {
        let active_candidate = nice_candidate();
        let candidate = uclamp_candidate("external-mutation-uclamp");
        let policy = policy(DaemonMode::Suggest);
        let mut observation = observation_with_active_nice(
            SituationKind::GameCpuSchedulerPressure,
            FocusGroupKind::Game,
            1234,
            0,
        );
        enable_capability_for_candidate(&mut observation, &candidate);

        let controller_state = ControllerRuntimeState {
            active_experiment: Some(ActiveExperiment {
                experiment_id: ExperimentId::new("external-mutation"),
                candidate: active_candidate,
                baseline_score_total: 1_000,
            }),
            ..ControllerRuntimeState::default()
        };
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
                .contains(&CandidateDenyReason::ExternalMutationDetected)
        );
        assert!(
            evaluation
                .deny_messages
                .iter()
                .any(|message| message.contains("restore or resync"))
        );
    }

    #[test]
    fn kept_action_no_longer_active_blocks_new_candidates_before_dry_run() {
        let kept_candidate = nice_candidate();
        let candidate = cpu_affinity_candidate("kept-no-longer-active-cpu-placement");
        let policy = policy(DaemonMode::Suggest);
        let observation = observation_with_active_nice(
            SituationKind::GameCpuSchedulerPressure,
            FocusGroupKind::Game,
            1234,
            0,
        );
        let active_profile_state = active_profile_state_with_kept(KeptCandidateState::new(
            ExperimentId::new("kept-no-longer-active"),
            kept_candidate,
            window_score(1_000),
            window_score(800),
            rollback_token(),
            observation.now_unix_nanos,
            "kept for test",
        ));
        let mut dry_runner = CountingDryRunner::default();
        let proposals = vec![proposal_for_candidate(candidate, 1.0)];

        let mut evaluations = super::super::super::evaluate_proposals_with_runner(
            PlannerInput {
                observation: &observation,
                daemon_policy: &policy,
                capabilities: &observation.capabilities,
                system_health: &observation.system_health,
                controller_state: &ControllerRuntimeState::default(),
                active_profile_state: Some(&active_profile_state),
                workload_policy: &WorkloadPolicyMatrix::default_rules(),
                profiles: &[],
            },
            proposals,
            &mut dry_runner,
        );

        assert_eq!(evaluations.len(), 1);
        let evaluation = evaluations.remove(0);
        assert_eq!(dry_runner.calls, 0);
        assert!(evaluation.dry_run.is_none());
        assert!(
            evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::KeptActionNoLongerActive)
        );
        assert!(
            evaluation
                .deny_messages
                .iter()
                .any(|message| message.contains("restore or resync"))
        );
    }

    #[test]
    fn active_cpu_placement_experiment_blocks_cgroup_placement_candidate() {
        let policy = suggest_policy_with_compile_cgroup();
        let mut registry = CandidateProviderRegistry::default();
        registry.register(Box::new(StaticProvider {
            candidate: cgroup_candidate("/user.slice/stutter-compile.slice"),
        }));
        let planner = CandidatePlanner::new(registry);
        let observation = observation();
        let mut capabilities = observation.capabilities.clone();
        capabilities.cgroup_v2_available = true;
        let controller_state = ControllerRuntimeState {
            active_experiment: Some(ActiveExperiment {
                experiment_id: ExperimentId::new("active-cpu-placement"),
                candidate: cpu_affinity_candidate("game-main"),
                baseline_score_total: 100,
            }),
            ..ControllerRuntimeState::default()
        };

        let result = planner.plan(PlannerInput {
            observation: &observation,
            daemon_policy: &policy,
            capabilities: &capabilities,
            system_health: &observation.system_health,
            controller_state: &controller_state,
            active_profile_state: None,
            workload_policy: &WorkloadPolicyMatrix::default_rules(),
            profiles: &[],
        });

        assert!(
            result.evaluations[0]
                .deny_reasons
                .contains(&CandidateDenyReason::ConflictWithActiveAction)
        );
    }

    #[test]
    fn independent_nice_candidate_does_not_conflict_with_kept_cpu_placement() {
        let policy = policy(DaemonMode::Suggest);
        let mut registry = CandidateProviderRegistry::default();
        registry.register(Box::new(StaticProvider {
            candidate: nice_candidate(),
        }));
        let planner = CandidatePlanner::new(registry);
        let observation = observation();
        let kept = KeptCandidateState::new(
            ExperimentId::new("kept-cpu-placement"),
            cpu_affinity_candidate("game-main"),
            window_score(100),
            window_score(80),
            rollback_token(),
            10,
            "kept test candidate",
        );
        let active_profile_state = active_profile_state_with_kept(kept);

        let result = planner.plan(PlannerInput {
            observation: &observation,
            daemon_policy: &policy,
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            controller_state: &ControllerRuntimeState::default(),
            active_profile_state: Some(&active_profile_state),
            workload_policy: &WorkloadPolicyMatrix::default_rules(),
            profiles: &[],
        });

        assert!(
            !result.evaluations[0]
                .deny_reasons
                .contains(&CandidateDenyReason::ConflictWithKeptAction)
        );
    }

    #[test]
    fn planner_checks_new_candidate_against_every_kept_action() {
        let policy = policy(DaemonMode::Suggest);
        let mut observation = observation();
        let candidate = cgroup_candidate("/user.slice/stutter-compile.slice");
        enable_capability_for_candidate(&mut observation, &candidate);
        let mut active_profile_state = ActiveProfileState::default();
        active_profile_state
            .record_kept_candidate(
                KeptCandidateState::new(
                    ExperimentId::new("kept-io-priority"),
                    ioprio_candidate("kept-io"),
                    window_score(100),
                    window_score(90),
                    rollback_token(),
                    10,
                    "kept compatible io priority",
                ),
                crate::autotune::comparison::ExperimentResult::Improved {
                    improvement_percent: 10.0,
                },
            )
            .unwrap();
        active_profile_state
            .record_kept_candidate(
                KeptCandidateState::new(
                    ExperimentId::new("kept-cpu-priority"),
                    nice_candidate(),
                    window_score(100),
                    window_score(90),
                    rollback_token(),
                    20,
                    "kept conflicting cpu priority",
                ),
                crate::autotune::comparison::ExperimentResult::Improved {
                    improvement_percent: 10.0,
                },
            )
            .unwrap();
        let mut dry_runner = CountingDryRunner::default();

        let mut evaluations = super::super::super::evaluate_proposals_with_runner(
            PlannerInput {
                observation: &observation,
                daemon_policy: &policy,
                capabilities: &observation.capabilities,
                system_health: &observation.system_health,
                controller_state: &ControllerRuntimeState::default(),
                active_profile_state: Some(&active_profile_state),
                workload_policy: &WorkloadPolicyMatrix::default_rules(),
                profiles: &[],
            },
            vec![proposal_for_candidate(candidate, 1.0)],
            &mut dry_runner,
        );

        let evaluation = evaluations.remove(0);
        assert_eq!(active_profile_state.kept_action_count(), 2);
        assert_eq!(dry_runner.calls, 0);
        assert!(
            evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::ConflictWithKeptAction)
        );
        assert!(
            evaluation
                .deny_messages
                .iter()
                .any(|message| message.contains("nice-background-compile"))
        );
    }
}
