//! Eligibility planner tests extracted from `autotune::planner`.
//!
//! Owns explicit workload/action eligibility tests outside the JSON golden fixture runner.
//! Does not own shared fixtures or production planner behavior.

#[cfg(test)]
mod tests {
    use super::super::{
        super::{CandidateDenyReason, CandidatePlanner, PlannerInput},
        support::*,
    };

    #[test]
    fn recording_situation_blocks_game_only_cpu_affinity_candidate() {
        let policy = policy(DaemonMode::Suggest);
        let mut registry = CandidateProviderRegistry::default();
        registry.register(Box::new(StaticProvider {
            candidate: cpu_affinity_candidate("game-main"),
        }));
        let planner = CandidatePlanner::new(registry);
        let mut observation = observation();
        observation.primary_situation = SituationKind::Recording;
        observation.focus_kind = Some(FocusGroupKind::Recording);
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

        assert!(
            result.evaluations[0]
                .deny_reasons
                .contains(&CandidateDenyReason::WorkloadPolicyBlocked)
        );
    }

    #[test]
    fn game_cpu_affinity_profile_stays_apply_low_risk_eligible() {
        let evaluation = evaluate_static_candidate(
            DaemonMode::ApplyLowRisk,
            SituationKind::GameCpuSchedulerPressure,
            FocusGroupKind::Game,
            cpu_affinity_candidate("game-main"),
        );

        assert!(
            evaluation.eligible,
            "expected cpu_affinity_profile to remain apply-low-risk eligible; deny_reasons={:?} deny_messages={:?}",
            evaluation.deny_reasons, evaluation.deny_messages
        );
        assert!(
            !evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::NotAutonomousForWorkload)
        );
    }

    #[test]
    fn game_uclamp_is_suggest_eligible_but_not_autonomous_for_apply() {
        let candidate = uclamp_candidate("game-uclamp");
        let suggest = evaluate_static_candidate(
            DaemonMode::Suggest,
            SituationKind::GameCpuSchedulerPressure,
            FocusGroupKind::Game,
            candidate.clone(),
        );

        assert!(
            suggest.eligible,
            "expected game uclamp to remain suggest eligible; deny_reasons={:?} deny_messages={:?}",
            suggest.deny_reasons, suggest.deny_messages
        );

        let apply = evaluate_static_candidate(
            DaemonMode::ApplyMediumRisk,
            SituationKind::GameCpuSchedulerPressure,
            FocusGroupKind::Game,
            candidate,
        );

        assert_autonomous_denial_mentions_context(
            &apply,
            SituationKind::GameCpuSchedulerPressure,
            "uclamp",
        );
    }

    #[test]
    fn compile_nice_is_suggest_eligible_but_not_autonomous_for_apply() {
        let candidate = nice_candidate();
        let suggest = evaluate_static_candidate(
            DaemonMode::Suggest,
            SituationKind::CompileCpuBound,
            FocusGroupKind::Compile,
            candidate.clone(),
        );

        assert!(
            suggest.eligible,
            "expected compile nice candidate to remain suggest eligible; deny_reasons={:?} deny_messages={:?}",
            suggest.deny_reasons, suggest.deny_messages
        );

        let apply = evaluate_static_candidate(
            DaemonMode::ApplyMediumRisk,
            SituationKind::CompileCpuBound,
            FocusGroupKind::Compile,
            candidate,
        );

        assert_autonomous_denial_mentions_context(&apply, SituationKind::CompileCpuBound, "nice");
    }

    #[test]
    fn empty_autonomous_families_block_browser_recording_irq_and_io_apply() {
        let cases = vec![
            (
                "browser nice",
                SituationKind::BrowserCpuPressure,
                FocusGroupKind::Browser,
                nice_candidate(),
            ),
            (
                "recording uclamp",
                SituationKind::Recording,
                FocusGroupKind::Recording,
                uclamp_candidate("recording-uclamp"),
            ),
            (
                "irq affinity",
                SituationKind::IrqPressure,
                FocusGroupKind::Desktop,
                irq_affinity_candidate("irq-affinity"),
            ),
            (
                "io ionice",
                SituationKind::IoPressure,
                FocusGroupKind::Desktop,
                ioprio_candidate("io-ionice"),
            ),
        ];

        for (label, situation, focus_kind, candidate) in cases {
            let action_kind = candidate.action_kind();
            let suggest = evaluate_static_candidate(
                DaemonMode::Suggest,
                situation,
                focus_kind,
                candidate.clone(),
            );

            if candidate.is_high_risk_system_adjacent() {
                assert!(!suggest.eligible);
                assert!(
                    suggest
                        .deny_reasons
                        .contains(&CandidateDenyReason::ManualOnlyHighRisk)
                );
            } else {
                assert!(
                    suggest.eligible,
                    "expected {label} to remain suggest eligible; deny_reasons={:?} deny_messages={:?}",
                    suggest.deny_reasons, suggest.deny_messages
                );
            }

            let apply = evaluate_static_candidate(
                DaemonMode::ApplyMediumRisk,
                situation,
                focus_kind,
                candidate,
            );

            assert_autonomous_denial_mentions_context(&apply, situation, action_kind);
        }
    }

    #[test]
    fn browser_foreground_blocks_compile_throughput_optimization() {
        let policy = suggest_policy_with_compile_cgroup();
        let mut registry = CandidateProviderRegistry::default();
        registry.register(Box::new(StaticProvider {
            candidate: cgroup_candidate("/user.slice/stutter-compile.slice"),
        }));
        let planner = CandidatePlanner::new(registry);
        let mut observation = observation();
        observation.primary_situation = SituationKind::BrowserFocused;
        observation.focus_kind = Some(FocusGroupKind::Browser);
        observation.refresh_situation_classification();
        let mut capabilities = observation.capabilities.clone();
        capabilities.cgroup_v2_available = true;

        let result = planner.plan(PlannerInput {
            observation: &observation,
            daemon_policy: &policy,
            capabilities: &capabilities,
            system_health: &observation.system_health,
            controller_state: &ControllerRuntimeState::default(),
            active_profile_state: None,
            workload_policy: &WorkloadPolicyMatrix::default_rules(),
            profiles: &[],
        });

        assert!(
            result.evaluations[0]
                .deny_reasons
                .contains(&CandidateDenyReason::WorkloadPolicyBlocked)
        );
        assert!(
            result.evaluations[0]
                .deny_reasons
                .contains(&CandidateDenyReason::ObjectiveNotAllowedForWorkload)
        );
    }
}
