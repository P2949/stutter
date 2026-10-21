use super::helpers::*;
use crate::{
    actions::{NiceAction, NicePolicy},
    autotune::{
        controller::*,
        decision::{AutotuneDecision, CandidateAction},
        experiment::WindowScore,
        objective::{ObjectiveKind, ObjectiveSignalQuality, ObjectiveSignals},
        planning::executable_plan::NiceActionPlan,
        state::AutotuneMode,
    },
};

fn candidate(objective: ObjectiveKind) -> CandidateAction {
    CandidateAction::Nice {
        plan: NiceActionPlan {
            name: "objective-contract".to_owned(),
            action: NiceAction {
                targets: Vec::new(),
                nice: 5,
                policy: NicePolicy::default(),
            },
            target_root_pid: Some(1234),
            evidence: Vec::new(),
            objective,
        },
    }
}

fn runnable(score: &WindowScore, p99: u64) -> ObjectiveSignals {
    let mut signals = ObjectiveSignals::from_window_score(score);
    signals.runnable_samples = Some(10_000);
    signals.runnable_latency_p99_ns = Some(p99);
    signals.runnable_latency_max_ns = Some(p99);
    signals.runnable_over_5ms = Some(0);
    signals.runnable_target_root_pid = Some(1234);
    signals.runnable_target_starttime_ticks = Some(100);
    signals.runnable_latency_source = Some("target-game-scheduler-histogram-v1".to_owned());
    signals.signal_quality.runnable_latency = ObjectiveSignalQuality::Direct;
    signals
}

#[test]
fn controller_uses_runnable_primary_in_both_generic_score_conflicts() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyMediumRisk);
    for (generic_total, baseline_p99, candidate_p99, expected_keep) in [
        (800, 2_000_000, 4_000_000, false),
        (1_200, 4_000_000, 2_000_000, true),
        (1_000, 4_000_000, 2_000_000, true),
    ] {
        let mut state = active_state_with_baseline_score(1_000);
        let active = state.active_experiment.as_mut().unwrap();
        active.candidate = candidate(ObjectiveKind::GameRunnableLatency);
        active.baseline_signals = runnable(&active.baseline_score, baseline_p99);
        let mut observation = high_quality_observation(generic_total);
        observation.objective_signals = runnable(&observation.to_window_score(), candidate_p99);
        let decision = decide_autotune_transition(&policy, &state, &observation, None);
        match decision {
            AutotuneDecision::KeepCurrent { reason, .. } if expected_keep => {
                assert!(reason.contains("primary_signal=runnable_p99_ns"))
            }
            AutotuneDecision::Revert { reason, .. } if !expected_keep => {
                assert!(reason.contains("primary_signal=runnable_p99_ns"))
            }
            other => panic!("unexpected objective transition: {other:?}"),
        }
    }
}

#[test]
fn completed_non_score_experiment_reverts_without_direct_evidence() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyMediumRisk);
    for objective in [
        ObjectiveKind::GameRunnableLatency,
        ObjectiveKind::CompileThroughputWithForegroundProtection,
    ] {
        let mut state = active_state_with_baseline_score(1_000);
        state.active_experiment.as_mut().unwrap().candidate = candidate(objective);
        let observation = high_quality_observation(800);
        match decide_autotune_transition(&policy, &state, &observation, None) {
            AutotuneDecision::Revert { reason, .. } => {
                assert!(reason.contains("outcome=inconclusive"))
            }
            other => panic!("missing primary must restore completed candidate: {other:?}"),
        }
    }
}
