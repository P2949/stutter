//! Policy and action-gate tests extracted from `autotune::apply_low_risk`.
//!
//! Owns low-risk action-kind and safety-class policy gates plus dry-run record eligibility tests.
//! Does not own target resolution, experiment resolution, audit/journal behavior, rollback orchestration, or production behavior.

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::super::*;
    use crate::{
        actions::{ActionId, ActionState, ActionWarning, SafetyClass},
        autotune::planning::{
            candidate::CandidateAction,
            dry_run::{CandidateDryRunRecord, dry_run_record_from_action_state},
        },
        profiles::Profile,
    };

    #[test]
    fn low_risk_candidate_executor_wraps_cpu_affinity_profile_candidates() {
        let profile = Profile {
            name: "game-main".to_owned(),
            rules: Vec::new(),
        };
        let candidate = CandidateAction::cpu_affinity_profile(profile, 4_242);

        let executor = CpuAffinityLowRiskExecutor::from_candidate(candidate.clone()).unwrap();
        assert_eq!(executor.candidate_name(), "game-main");
        assert_eq!(executor.action_kind(), "cpu_affinity_profile");
        assert_eq!(executor.safety_class(), SafetyClass::ReversibleLowRisk);

        let boxed = executor_for_low_risk_candidate(candidate).unwrap();
        assert_eq!(boxed.candidate_name(), "game-main");
        assert_eq!(boxed.action_kind(), "cpu_affinity_profile");
        assert_eq!(boxed.safety_class(), SafetyClass::ReversibleLowRisk);
    }

    #[tokio::test]
    async fn run_apply_low_risk_candidate_rejects_non_profile_candidates() {
        let err = run_apply_low_risk_candidate(
            CandidateAction::fake(
                ActionId::new("fake-low-risk".to_owned()),
                SafetyClass::ReversibleLowRisk,
            ),
            Duration::ZERO,
        )
        .await
        .unwrap_err()
        .to_string();

        assert!(err.contains("supports CPU-affinity profile actions only"));
    }

    #[tokio::test]
    async fn autotune_apply_low_risk_cannot_apply_medium_candidate() {
        let mut executor = FakeExecutor::low_risk();
        executor.safety_class = SafetyClass::ReversibleMediumRisk;

        let err = run_apply_low_risk_with_executor(&mut executor, Duration::ZERO)
            .await
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("apply-low-risk currently supports")
        );
        assert!(err.to_string().contains("ReversibleLowRisk"));
        assert_eq!(executor.dry_run_calls, 0);
        assert_eq!(executor.apply_calls, 0);
        assert_eq!(executor.rollback_calls, 0);
    }

    #[tokio::test]
    async fn high_risk_action_is_blocked_before_dry_run_or_apply() {
        let mut executor = FakeExecutor::low_risk();
        executor.safety_class = SafetyClass::HighRisk;

        let err = run_apply_low_risk_with_executor(&mut executor, Duration::ZERO)
            .await
            .unwrap_err()
            .to_string();

        assert!(
            err.contains("currently supports ReversibleLowRisk CPU-affinity profile actions only")
        );
        assert_eq!(executor.dry_run_calls, 0);
        assert_eq!(executor.apply_calls, 0);
        assert_eq!(executor.rollback_calls, 0);
    }

    #[tokio::test]
    async fn non_cpu_affinity_action_is_blocked_before_dry_run_or_apply() {
        let mut executor = FakeExecutor::low_risk();
        executor.action_kind = "gpu_power_profile";

        let err = run_apply_low_risk_with_executor(&mut executor, Duration::ZERO)
            .await
            .unwrap_err()
            .to_string();

        assert!(err.contains("currently supports CPU-affinity profile actions only"));
        assert_eq!(executor.dry_run_calls, 0);
        assert_eq!(executor.apply_calls, 0);
        assert_eq!(executor.rollback_calls, 0);
    }

    #[tokio::test]
    async fn zero_affected_tasks_are_blocked_before_apply() {
        let mut executor = FakeExecutor::low_risk();
        executor.dry_run_record = Some(CandidateDryRunRecord {
            candidate_name: "zero".to_owned(),
            affected_tasks: 0,
            warnings: Vec::new(),
            safety_class: SafetyClass::ReversibleLowRisk,
            eligible: false,
            reason: Some("dry-run matched zero affected tasks".to_owned()),
        });

        let err = run_apply_low_risk_with_executor(&mut executor, Duration::ZERO)
            .await
            .unwrap_err()
            .to_string();

        assert!(err.contains("not eligible"));
        assert_eq!(executor.dry_run_calls, 1);
        assert_eq!(executor.apply_calls, 0);
        assert_eq!(executor.rollback_calls, 0);
    }

    #[test]
    fn dry_run_warning_is_preserved_in_record() {
        let state = ActionState {
            applied: false,
            affected_tasks: 31,
            checked_tasks: 31,
            pending_changes: 31,
            warnings: vec![ActionWarning {
                message: "restore file already exists".to_owned(),
            }],
        };

        let record = dry_run_record_from_action_state(
            "warned".to_owned(),
            SafetyClass::ReversibleLowRisk,
            state,
        );

        assert!(record.eligible);
        assert_eq!(record.warnings.len(), 1);
        assert_eq!(record.warnings[0].message, "restore file already exists");
    }

    #[test]
    fn apply_low_risk_rejects_medium_risk_profile_action() {
        let err = ensure_low_risk_action_allowed(
            "cpu_affinity_profile",
            &SafetyClass::ReversibleMediumRisk,
        )
        .unwrap_err()
        .to_string();

        assert!(err.contains("ReversibleLowRisk"));
        assert!(err.contains("ReversibleMediumRisk"));
    }
}
