//! Ranking planner tests extracted from `autotune::planner`.
//!
//! Owns planner candidate ordering tests.
//! Does not own shared fixtures or production planner behavior.

#[cfg(test)]
mod tests {
    use super::super::{
        super::{PlannerInput, evaluate_proposals_with_runner, sort_candidate_evaluations},
        support::*,
    };

    #[test]
    fn equal_rank_candidates_sort_by_higher_confidence_before_candidate_name() {
        let low_confidence = cpu_affinity_candidate("aaa-low-confidence");
        let high_confidence = cpu_affinity_candidate("zzz-high-confidence");
        let policy = policy(DaemonMode::Suggest);
        let observation = observation();
        let mut dry_runner = CountingDryRunner::default();
        let proposals = vec![
            CandidateProposal {
                candidate: low_confidence.clone(),
                provider: low_confidence.action_kind(),
                confidence: 0.80,
                deny_reasons: Vec::new(),
                objective: low_confidence.objective(),
                rank_hint: 1,
            },
            CandidateProposal {
                candidate: high_confidence.clone(),
                provider: high_confidence.action_kind(),
                confidence: 0.90,
                deny_reasons: Vec::new(),
                objective: high_confidence.objective(),
                rank_hint: 1,
            },
        ];

        let mut evaluations = evaluate_proposals_with_runner(
            PlannerInput {
                observation: &observation,
                daemon_policy: &policy,
                capabilities: &observation.capabilities,
                system_health: &observation.system_health,
                controller_state: &ControllerRuntimeState::default(),
                active_profile_state: None,
                workload_policy: &WorkloadPolicyMatrix::default_rules(),
                profiles: &[],
            },
            proposals,
            &mut dry_runner,
        );
        sort_candidate_evaluations(&mut evaluations);

        assert_eq!(dry_runner.calls, 2);
        assert_eq!(evaluations[0].candidate_name, "zzz-high-confidence");
        assert_eq!(evaluations[0].confidence, 0.90);
        assert_eq!(evaluations[1].candidate_name, "aaa-low-confidence");
        assert_eq!(evaluations[1].confidence, 0.80);
    }
}
