//! Summary planner tests extracted from `autotune::planner`.
//!
//! Owns planner summary and manual-only selected-summary tests.
//! Does not own shared fixtures or production planner behavior.

#[cfg(test)]
mod tests {
    use super::super::{
        super::{CandidatePlanner, PlannerInput},
        support::*,
    };

    #[test]
    fn planner_summary_groups_denials_and_manual_only_suggestions() {
        let policy = policy(DaemonMode::ApplyLowRisk);
        let mut registry = CandidateProviderRegistry::default();
        registry.register(Box::new(StaticProvider {
            candidate: high_risk_irq_affinity_candidate("irq-manual"),
        }));
        let planner = CandidatePlanner::new(registry);
        let mut observation = observation();
        observation.primary_situation = SituationKind::IrqPressure;
        observation.focus_kind = Some(FocusGroupKind::Game);
        observation.capabilities.irq_affinity_available = true;
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
        let summary = result.summary();

        assert_eq!(summary.total_proposals, 1);
        assert_eq!(summary.eligible_proposals, 0);
        assert!(summary.eligible_candidates.is_empty());
        assert_eq!(summary.top_denied_candidates.len(), 1);
        assert!(
            summary.top_denied_candidates[0]
                .evidence
                .iter()
                .any(|evidence| evidence.contains("situation="))
        );
        assert!(
            summary
                .grouped_denials
                .iter()
                .any(|denial| denial.reason_code == "manual_only_high_risk")
        );
        assert_eq!(summary.manual_only_suggestions, vec!["irq-manual"]);
        assert!(summary.no_action.is_some());
    }

    #[test]
    fn manual_only_high_risk_candidate_is_never_selected_for_apply_modes() {
        for mode in [DaemonMode::ApplyLowRisk, DaemonMode::ApplyMediumRisk] {
            let candidate_name = format!("irq-manual-{}", mode.as_str());
            let policy = policy(mode);
            let mut registry = CandidateProviderRegistry::default();
            registry.register(Box::new(StaticProvider {
                candidate: high_risk_irq_affinity_candidate(&candidate_name),
            }));
            let planner = CandidatePlanner::new(registry);
            let mut observation = observation();
            observation.primary_situation = SituationKind::IrqPressure;
            observation.focus_kind = Some(FocusGroupKind::Game);
            observation.capabilities.irq_affinity_available = true;
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
            let summary = result.summary();

            assert!(
                result.selected.is_none(),
                "manual-only candidate was selected in mode {mode}"
            );
            assert_eq!(summary.total_proposals, 1);
            assert_eq!(summary.eligible_proposals, 0);
            assert_eq!(summary.manual_only_suggestions, vec![candidate_name]);
            assert!(
                summary
                    .grouped_denials
                    .iter()
                    .any(|denial| denial.reason_code == "manual_only_high_risk")
            );
        }
    }
}
