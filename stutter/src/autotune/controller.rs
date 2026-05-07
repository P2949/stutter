#![allow(dead_code)]

use std::time::Duration;

use super::{
    decision::{AutotuneDecision, CandidateAction, ExperimentId},
    observation::AutotuneObservation,
    quality::OnlineDataQuality,
    state::{AutotuneMode, ControllerPhase},
};
use crate::actions::SafetyClass;

#[derive(Clone, Debug)]
pub struct ControllerPolicy {
    pub mode: AutotuneMode,
    pub max_safety_class: SafetyClass,
    pub min_improvement_percent: f64,
    pub max_regression_percent: f64,
    pub cooldown: Duration,
}

impl ControllerPolicy {
    pub fn for_mode(mode: AutotuneMode) -> Self {
        let max_safety_class = match mode {
            AutotuneMode::Observe | AutotuneMode::Suggest => SafetyClass::ObserveOnly,
            AutotuneMode::ApplyLowRisk => SafetyClass::ReversibleLowRisk,
            AutotuneMode::ApplyMediumRisk => SafetyClass::ReversibleMediumRisk,
            AutotuneMode::ApplyHighRisk => SafetyClass::HighRisk,
        };

        Self {
            mode,
            max_safety_class,
            min_improvement_percent: 12.5,
            max_regression_percent: 7.5,
            cooldown: Duration::from_secs(60),
        }
    }

    pub fn can_start_experiment(&self) -> bool {
        matches!(
            self.mode,
            AutotuneMode::ApplyLowRisk
                | AutotuneMode::ApplyMediumRisk
                | AutotuneMode::ApplyHighRisk
        )
    }
}

#[derive(Clone, Debug)]
pub struct ActiveExperiment {
    pub experiment_id: ExperimentId,
    pub candidate: CandidateAction,
    pub baseline_score_total: u64,
}

#[derive(Clone, Debug)]
pub struct ControllerRuntimeState {
    pub phase: ControllerPhase,
    pub active_experiment: Option<ActiveExperiment>,
    pub cooldown_until_unix_nanos: Option<u128>,
}

impl Default for ControllerRuntimeState {
    fn default() -> Self {
        Self {
            phase: ControllerPhase::Observing,
            active_experiment: None,
            cooldown_until_unix_nanos: None,
        }
    }
}

pub fn decide_autotune_transition(
    policy: &ControllerPolicy,
    state: &ControllerRuntimeState,
    observation: &AutotuneObservation,
    candidate: Option<CandidateAction>,
) -> AutotuneDecision {
    if state.phase == ControllerPhase::Disabled {
        return AutotuneDecision::Noop {
            reason: "controller is disabled".to_owned(),
        };
    }

    if let Some(active) = &state.active_experiment {
        if !observation.target_present {
            return AutotuneDecision::Revert {
                experiment_id: active.experiment_id.clone(),
                reason: "target disappeared during active experiment".to_owned(),
            };
        }

        if let OnlineDataQuality::Low { reasons } = &observation.data_quality {
            return AutotuneDecision::Revert {
                experiment_id: active.experiment_id.clone(),
                reason: format!(
                    "low data quality during active experiment: {}",
                    reasons.join("; ")
                ),
            };
        }

        let regression_percent =
            score_regression_percent(active.baseline_score_total, observation.score.total);
        if regression_percent > policy.max_regression_percent {
            return AutotuneDecision::Revert {
                experiment_id: active.experiment_id.clone(),
                reason: format!(
                    "candidate regressed score by {:.1}% which exceeds max_regression_percent {:.1}%",
                    regression_percent, policy.max_regression_percent
                ),
            };
        }

        let improvement_percent =
            score_improvement_percent(active.baseline_score_total, observation.score.total);
        if improvement_percent >= policy.min_improvement_percent {
            return AutotuneDecision::KeepCurrent {
                experiment_id: active.experiment_id.clone(),
                reason: format!(
                    "candidate improved score by {:.1}% which meets min_improvement_percent {:.1}%",
                    improvement_percent, policy.min_improvement_percent
                ),
            };
        }

        return AutotuneDecision::Noop {
            reason: format!(
                "active experiment is still inconclusive: improvement={:.1}% regression={:.1}%",
                improvement_percent, regression_percent
            ),
        };
    }

    if let Some(cooldown_until) = state.cooldown_until_unix_nanos
        && observation.now_unix_nanos < cooldown_until
    {
        let remaining_nanos = cooldown_until.saturating_sub(observation.now_unix_nanos);
        return AutotuneDecision::EnterCooldown {
            duration: Duration::from_nanos(remaining_nanos.min(u64::MAX as u128) as u64),
            reason: "cooldown blocks repeated action".to_owned(),
        };
    }

    if policy.mode == AutotuneMode::Observe {
        return AutotuneDecision::Noop {
            reason: "observe mode never applies or suggests actions".to_owned(),
        };
    }

    if let OnlineDataQuality::Low { reasons } = &observation.data_quality {
        return AutotuneDecision::Noop {
            reason: format!("low data quality blocks experiment: {}", reasons.join("; ")),
        };
    }

    let Some(candidate) = candidate else {
        return AutotuneDecision::Noop {
            reason: "no candidate action available".to_owned(),
        };
    };

    if candidate.safety_class() > policy.max_safety_class {
        return AutotuneDecision::Noop {
            reason: format!(
                "candidate safety class {:?} exceeds mode maximum {:?}",
                candidate.safety_class(),
                policy.max_safety_class
            ),
        };
    }

    if policy.mode == AutotuneMode::Suggest {
        return AutotuneDecision::Suggest {
            candidate,
            reason: "suggest mode reports candidate without applying".to_owned(),
        };
    }

    if !policy.can_start_experiment() {
        return AutotuneDecision::Noop {
            reason: "current mode is not allowed to start experiments".to_owned(),
        };
    }

    AutotuneDecision::StartExperiment {
        candidate,
        reason: "candidate passed data-quality and safety gates".to_owned(),
    }
}

fn score_regression_percent(baseline: u64, current: u64) -> f64 {
    if current <= baseline {
        0.0
    } else if baseline == 0 {
        100.0
    } else {
        ((current - baseline) as f64 / baseline as f64) * 100.0
    }
}

fn score_improvement_percent(baseline: u64, current: u64) -> f64 {
    if current >= baseline || baseline == 0 {
        0.0
    } else {
        ((baseline - current) as f64 / baseline as f64) * 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ActiveExperiment, ControllerPolicy, ControllerRuntimeState, decide_autotune_transition,
    };
    use crate::{
        actions::{ActionId, SafetyClass},
        autotune::{
            decision::{AutotuneDecision, CandidateAction, ExperimentId},
            observation::AutotuneObservation,
            quality::OnlineDataQuality,
            state::{AutotuneMode, ControllerPhase, SituationKind},
        },
        scorer::StutterScore,
    };

    fn candidate_with_safety_class(safety_class: SafetyClass) -> CandidateAction {
        CandidateAction::Fake {
            action_id: ActionId("test".to_owned()),
            safety_class,
        }
    }

    fn high_quality_observation(score_total: u64) -> AutotuneObservation {
        AutotuneObservation {
            now_unix_nanos: 1_000_000_000,
            elapsed_ms: 30_000,
            target_present: true,
            target_root_pid: Some(1234),
            active_target_count: 4,
            scored_task_count: 2,
            interval_count: 5,
            scored_samples: 100,
            score: StutterScore {
                total: score_total,
                ..StutterScore::default()
            },
            data_quality: OnlineDataQuality::High,
            primary_situation: SituationKind::GameCpuSchedulerPressure,
            recent_diagnoses: Vec::new(),
            frame_count: 100,
            frame_p99_ms: 12.0,
            frame_max_ms: 20.0,
            drop_counter_total: 0,
        }
    }

    fn active_state_with_baseline_score(baseline_score_total: u64) -> ControllerRuntimeState {
        ControllerRuntimeState {
            phase: ControllerPhase::Measuring,
            active_experiment: Some(ActiveExperiment {
                experiment_id: ExperimentId("experiment-1".to_owned()),
                candidate: candidate_with_safety_class(SafetyClass::ReversibleLowRisk),
                baseline_score_total,
            }),
            cooldown_until_unix_nanos: None,
        }
    }

    #[test]
    fn observe_mode_never_applies() {
        let policy = ControllerPolicy::for_mode(AutotuneMode::Observe);
        let state = ControllerRuntimeState::default();
        let observation = high_quality_observation(100);
        let candidate = candidate_with_safety_class(SafetyClass::ReversibleLowRisk);

        let decision = decide_autotune_transition(&policy, &state, &observation, Some(candidate));

        match decision {
            AutotuneDecision::Noop { reason } => {
                assert!(reason.contains("observe mode never applies"));
            }
            other => panic!("expected Noop, got {other:?}"),
        }
    }

    #[test]
    fn low_data_quality_blocks_experiment() {
        let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
        let state = ControllerRuntimeState::default();
        let mut observation = high_quality_observation(100);
        observation.data_quality = OnlineDataQuality::Low {
            reasons: vec!["fewer than min_scored_samples".to_owned()],
        };
        let candidate = candidate_with_safety_class(SafetyClass::ReversibleLowRisk);

        let decision = decide_autotune_transition(&policy, &state, &observation, Some(candidate));

        match decision {
            AutotuneDecision::Noop { reason } => {
                assert!(reason.contains("low data quality blocks experiment"));
                assert!(reason.contains("fewer than min_scored_samples"));
            }
            other => panic!("expected Noop, got {other:?}"),
        }
    }

    #[test]
    fn high_risk_action_blocked_by_low_risk_mode() {
        let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
        let state = ControllerRuntimeState::default();
        let observation = high_quality_observation(100);
        let candidate = candidate_with_safety_class(SafetyClass::HighRisk);

        let decision = decide_autotune_transition(&policy, &state, &observation, Some(candidate));

        match decision {
            AutotuneDecision::Noop { reason } => {
                assert!(reason.contains("exceeds mode maximum"));
                assert!(reason.contains("HighRisk"));
                assert!(reason.contains("ReversibleLowRisk"));
            }
            other => panic!("expected Noop, got {other:?}"),
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
                assert!(reason.contains("regressed score"));
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
                assert!(reason.contains("improved score"));
                assert!(reason.contains("meets min_improvement_percent"));
            }
            other => panic!("expected KeepCurrent, got {other:?}"),
        }
    }

    #[test]
    fn cooldown_blocks_repeated_action() {
        let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
        let mut observation = high_quality_observation(100);
        observation.now_unix_nanos = 1_000_000_000;
        let state = ControllerRuntimeState {
            phase: ControllerPhase::Cooldown,
            active_experiment: None,
            cooldown_until_unix_nanos: Some(11_000_000_000),
        };
        let candidate = candidate_with_safety_class(SafetyClass::ReversibleLowRisk);

        let decision = decide_autotune_transition(&policy, &state, &observation, Some(candidate));

        match decision {
            AutotuneDecision::EnterCooldown { duration, reason } => {
                assert_eq!(duration.as_secs(), 10);
                assert!(reason.contains("cooldown blocks repeated action"));
            }
            other => panic!("expected EnterCooldown, got {other:?}"),
        }
    }
}
