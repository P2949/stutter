use super::helpers::*;
use crate::autotune::{
    controller::*,
    decision::{AutotuneDecision, ExperimentId},
    quality::OnlineDataQuality,
    state::AutotuneMode,
};

#[test]
fn active_experiment_does_not_revert_when_candidate_raw_score_is_higher_but_rate_is_equal() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
    let state = active_state_with_baseline_window(1_000, 100, 5); // 10.0 / sample
    let observation = high_quality_observation_with_score(3_000, 300, 15); // 10.0 / sample

    let decision = decide_autotune_transition(&policy, &state, &observation, None);

    match decision {
        AutotuneDecision::Noop { reason } => {
            assert!(
                reason.contains("active experiment is still inconclusive"),
                "expected Noop due to inconclusive normalized rates, got: {reason}"
            );
        }
        other => panic!("expected Noop, got {other:?}"),
    }
}

#[test]
fn active_experiment_reverts_when_candidate_raw_score_is_lower_but_rate_is_worse() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
    let state = active_state_with_baseline_window(1_000, 1_000, 50); // 1.0 / sample
    let observation = high_quality_observation_with_score(900, 100, 5); // 9.0 / sample

    let decision = decide_autotune_transition(&policy, &state, &observation, None);

    match decision {
        AutotuneDecision::Revert {
            experiment_id,
            reason,
        } => {
            assert_eq!(experiment_id, ExperimentId("experiment-1".to_owned()));
            assert!(
                reason.contains("regressed normalized score"),
                "expected regression, got: {reason}"
            );
        }
        other => {
            panic!("expected Revert for worse rate despite lower raw score, got {other:?}")
        }
    }
}

#[test]
fn active_experiment_keeps_when_candidate_raw_score_is_higher_but_rate_is_better() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
    let state = active_state_with_baseline_window(1_000, 100, 5); // 10.0 / sample
    let observation = high_quality_observation_with_score(1_500, 300, 15); // 5.0 / sample

    let decision = decide_autotune_transition(&policy, &state, &observation, None);

    match decision {
        AutotuneDecision::KeepCurrent {
            experiment_id,
            reason,
        } => {
            assert_eq!(experiment_id, ExperimentId("experiment-1".to_owned()));
            assert!(
                reason.contains("improved normalized score"),
                "expected improvement, got: {reason}"
            );
        }
        other => {
            panic!("expected KeepCurrent for better rate despite higher raw score, got {other:?}")
        }
    }
}

#[test]
fn active_experiment_reverts_when_score_comparison_is_invalid() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
    let state = active_state_with_baseline_window(1_000, 100, 5);
    let mut observation = high_quality_observation_with_score(1_000, 0, 0);
    observation.scored_task_count = 0;

    let decision = decide_autotune_transition(&policy, &state, &observation, None);

    match decision {
        AutotuneDecision::Revert {
            experiment_id,
            reason,
        } => {
            assert_eq!(experiment_id, ExperimentId("experiment-1".to_owned()));
            assert!(
                reason.contains("invalid active experiment score comparison"),
                "expected invalid reason, got: {reason}"
            );
        }
        other => panic!("expected Revert for invalid score comparison, got {other:?}"),
    }
}

#[test]
fn target_disappeared_reverts_active_experiment() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
    let state = active_state_with_baseline_score(100);
    let mut observation = high_quality_observation(90);
    observation.target_present = false;
    observation.data_quality = OnlineDataQuality::Low {
        reasons: vec!["target disappeared".to_owned()],
    };

    let decision = decide_autotune_transition(&policy, &state, &observation, None);

    match decision {
        AutotuneDecision::Revert {
            experiment_id,
            reason,
        } => {
            assert_eq!(experiment_id, ExperimentId("experiment-1".to_owned()));
            assert!(reason.contains("target disappeared"));
        }
        other => panic!("expected Revert, got {other:?}"),
    }
}

#[test]
fn regression_reverts_candidate() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
    let state = active_state_with_baseline_score(100);
    let observation = high_quality_observation(120);

    let decision = decide_autotune_transition(&policy, &state, &observation, None);

    match decision {
        AutotuneDecision::Revert {
            experiment_id,
            reason,
        } => {
            assert_eq!(experiment_id, ExperimentId("experiment-1".to_owned()));
            assert!(reason.contains("regressed normalized score"));
            assert!(reason.contains("exceeds max_regression_percent"));
        }
        other => panic!("expected Revert, got {other:?}"),
    }
}

#[test]
fn improvement_keeps_candidate() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
    let state = active_state_with_baseline_score(100);
    let observation = high_quality_observation(80);

    let decision = decide_autotune_transition(&policy, &state, &observation, None);

    match decision {
        AutotuneDecision::KeepCurrent {
            experiment_id,
            reason,
        } => {
            assert_eq!(experiment_id, ExperimentId("experiment-1".to_owned()));
            assert!(reason.contains("improved normalized score"));
            assert!(reason.contains("meets min_improvement_percent"));
        }
        other => panic!("expected KeepCurrent, got {other:?}"),
    }
}
