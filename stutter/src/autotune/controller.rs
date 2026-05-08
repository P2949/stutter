#![allow(dead_code)]

use std::time::Duration;

use super::{
    decision::{AutotuneDecision, CandidateAction, ExperimentId},
    observation::AutotuneObservation,
    quality::OnlineDataQuality,
    state::{AutotuneMode, ControllerPhase},
};
use crate::{
    actions::{ActionId, SafetyClass},
    focus::FocusGroupKind,
    process_tree::TaskClass,
};

#[derive(Clone, Debug)]
pub struct ControllerPolicy {
    pub mode: AutotuneMode,
    pub max_safety_class: SafetyClass,
    pub min_improvement_percent: f64,
    pub max_regression_percent: f64,
    pub cooldown_after_keep: Duration,
    pub cooldown_after_revert: Duration,
    pub cooldown_after_fault: Duration,
    pub minimum_time_between_same_action: Duration,
}

impl ControllerPolicy {
    pub fn for_mode(mode: AutotuneMode) -> Self {
        let max_safety_class = match mode {
            AutotuneMode::Observe => SafetyClass::ObserveOnly,
            AutotuneMode::Suggest => SafetyClass::HighRisk,
            AutotuneMode::ApplyLowRisk => SafetyClass::ReversibleLowRisk,
            AutotuneMode::ApplyMediumRisk => SafetyClass::ReversibleMediumRisk,
            AutotuneMode::ApplyHighRisk => SafetyClass::HighRisk,
        };

        Self {
            mode,
            max_safety_class,
            min_improvement_percent: 12.5,
            max_regression_percent: 7.5,
            cooldown_after_keep: Duration::from_secs(60),
            cooldown_after_revert: Duration::from_secs(120),
            cooldown_after_fault: Duration::from_secs(300),
            minimum_time_between_same_action: Duration::from_secs(300),
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
    pub last_action_at_unix_nanos: Vec<(ActionId, u128)>,
}

impl Default for ControllerRuntimeState {
    fn default() -> Self {
        Self {
            phase: ControllerPhase::Observing,
            active_experiment: None,
            cooldown_until_unix_nanos: None,
            last_action_at_unix_nanos: Vec::new(),
        }
    }
}

impl ControllerRuntimeState {
    pub fn mark_candidate_action_attempted(
        &mut self,
        candidate: &CandidateAction,
        now_unix_nanos: u128,
    ) {
        let action_id = candidate.action_id();

        if let Some((_, last_at)) = self
            .last_action_at_unix_nanos
            .iter_mut()
            .find(|(existing, _)| existing == &action_id)
        {
            *last_at = now_unix_nanos;
            return;
        }

        self.last_action_at_unix_nanos
            .push((action_id, now_unix_nanos));
    }

    pub fn enter_cooldown_after_keep(&mut self, policy: &ControllerPolicy, now_unix_nanos: u128) {
        self.phase = ControllerPhase::Cooldown;
        self.cooldown_until_unix_nanos =
            cooldown_deadline(now_unix_nanos, policy.cooldown_after_keep);
    }

    pub fn enter_cooldown_after_revert(&mut self, policy: &ControllerPolicy, now_unix_nanos: u128) {
        self.phase = ControllerPhase::Cooldown;
        self.active_experiment = None;
        self.cooldown_until_unix_nanos =
            cooldown_deadline(now_unix_nanos, policy.cooldown_after_revert);
    }

    pub fn enter_cooldown_after_fault(&mut self, policy: &ControllerPolicy, now_unix_nanos: u128) {
        self.phase = ControllerPhase::Faulted;
        self.active_experiment = None;
        self.cooldown_until_unix_nanos =
            cooldown_deadline(now_unix_nanos, policy.cooldown_after_fault);
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

    if state.phase == ControllerPhase::Faulted {
        if let Some(decision) = cooldown_block_decision(
            state,
            observation,
            "fault cooldown blocks repeated action after controller fault",
        ) {
            return decision;
        }

        return AutotuneDecision::Fault {
            reason: "controller is faulted; manual intervention required".to_owned(),
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

        if let Some(reason) = focus_policy_block_reason(observation, &active.candidate) {
            return AutotuneDecision::Revert {
                experiment_id: active.experiment_id.clone(),
                reason: format!("focus policy blocks active experiment: {reason}"),
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

    if let Some(decision) =
        cooldown_block_decision(state, observation, "cooldown blocks repeated action")
    {
        return decision;
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

    if let Some(reason) = focus_policy_block_reason(observation, &candidate) {
        return AutotuneDecision::Noop { reason };
    }

    if candidate.safety_class() > policy.max_safety_class {
        return AutotuneDecision::Noop {
            reason: format!(
                "candidate safety class {:?} exceeds mode maximum {:?}",
                candidate.safety_class(),
                policy.max_safety_class
            ),
        };
    }

    if let Some(duration) = same_action_cooldown_remaining(policy, state, observation, &candidate) {
        return AutotuneDecision::EnterCooldown {
            duration,
            reason: format!(
                "same action '{}' is still cooling down; minimum_time_between_same_action is {}s",
                candidate.action_id().0,
                policy.minimum_time_between_same_action.as_secs()
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

fn cooldown_block_decision(
    state: &ControllerRuntimeState,
    observation: &AutotuneObservation,
    reason: &str,
) -> Option<AutotuneDecision> {
    let cooldown_until = state.cooldown_until_unix_nanos?;

    if observation.now_unix_nanos >= cooldown_until {
        return None;
    }

    let remaining_nanos = cooldown_until.saturating_sub(observation.now_unix_nanos);
    Some(AutotuneDecision::EnterCooldown {
        duration: duration_from_remaining_nanos(remaining_nanos),
        reason: reason.to_owned(),
    })
}

fn same_action_cooldown_remaining(
    policy: &ControllerPolicy,
    state: &ControllerRuntimeState,
    observation: &AutotuneObservation,
    candidate: &CandidateAction,
) -> Option<Duration> {
    if policy.minimum_time_between_same_action.is_zero() {
        return None;
    }

    let action_id = candidate.action_id();
    let (_, last_at) = state
        .last_action_at_unix_nanos
        .iter()
        .find(|(existing, _)| existing == &action_id)?;

    let next_allowed = last_at.saturating_add(policy.minimum_time_between_same_action.as_nanos());
    if observation.now_unix_nanos >= next_allowed {
        return None;
    }

    Some(duration_from_remaining_nanos(
        next_allowed.saturating_sub(observation.now_unix_nanos),
    ))
}

fn cooldown_deadline(now_unix_nanos: u128, duration: Duration) -> Option<u128> {
    if duration.is_zero() {
        None
    } else {
        Some(now_unix_nanos.saturating_add(duration.as_nanos()))
    }
}

fn duration_from_remaining_nanos(remaining_nanos: u128) -> Duration {
    Duration::from_nanos(remaining_nanos.min(u64::MAX as u128) as u64)
}

fn focus_policy_block_reason(
    observation: &AutotuneObservation,
    candidate: &CandidateAction,
) -> Option<String> {
    if observation.focus_has_critical_realtime_warning()
        && candidate.safety_class() > SafetyClass::ObserveOnly
    {
        return Some(
            "critical realtime/input process is present in the focus context; unsafe action classes are blocked"
                .to_owned(),
        );
    }

    if observation.focus_is_idle_or_unknown() {
        return Some("focus is idle or unknown; autotune remains observe-only".to_owned());
    }

    match observation.focus_kind {
        Some(FocusGroupKind::Game) => None,
        Some(FocusGroupKind::Compile) => {
            if candidate_looks_like_game_cpu_isolation_profile(candidate) {
                Some(
                    "compile focus must not apply gaming CPU-isolation profile candidates"
                        .to_owned(),
                )
            } else {
                None
            }
        }
        Some(FocusGroupKind::Browser) => {
            if candidate_looks_like_game_cpu_isolation_profile(candidate) {
                Some(
                    "browser focus avoids gaming CPU-isolation profiles that may hurt desktop responsiveness"
                        .to_owned(),
                )
            } else {
                None
            }
        }
        Some(FocusGroupKind::Idle) | Some(FocusGroupKind::Unknown) | None => {
            Some("focus is idle or unknown; autotune remains observe-only".to_owned())
        }
        Some(FocusGroupKind::Desktop)
        | Some(FocusGroupKind::Media)
        | Some(FocusGroupKind::Recording)
        | Some(FocusGroupKind::VirtualMachine) => None,
    }
}

fn candidate_looks_like_game_cpu_isolation_profile(candidate: &CandidateAction) -> bool {
    match candidate {
        CandidateAction::CpuAffinityProfile {
            profile_name,
            profile,
            ..
        } => {
            let lower_name = profile_name.to_ascii_lowercase();
            lower_name.contains("game")
                || lower_name.contains("gaming")
                || lower_name.contains("isolation")
                || profile.rules.iter().any(|rule| {
                    rule.match_class.iter().any(|class| {
                        matches!(
                            class,
                            TaskClass::Game
                                | TaskClass::GameHelper
                                | TaskClass::GameRenderThread
                                | TaskClass::GameWorkerThread
                                | TaskClass::WineServer
                                | TaskClass::GameScope
                        )
                    })
                })
        }
        #[cfg(test)]
        CandidateAction::Fake { .. } => false,
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
    use super::*;
    use crate::{
        actions::{ActionId, SafetyClass},
        affinity::CpuMask,
        autotune::{
            decision::{AutotuneDecision, CandidateAction, ExperimentId},
            observation::AutotuneObservation,
            quality::OnlineDataQuality,
            state::{AutotuneMode, ControllerPhase, SituationKind},
        },
        focus::FocusGroupKind,
        process_tree::TaskClass,
        profiles::{Profile, ProfileRule},
        scorer::StutterScore,
    };

    fn gaming_cpu_affinity_candidate() -> CandidateAction {
        CandidateAction::cpu_affinity_profile(
            Profile {
                name: "game-main-suggested".to_owned(),
                rules: vec![ProfileRule {
                    affinity: CpuMask::parse("0").unwrap(),
                    match_class: vec![TaskClass::Game],
                    match_comm: Vec::new(),
                }],
            },
            1234,
        )
    }

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
            focus_kind: Some(FocusGroupKind::Game),
            focus_confidence: 0.80,
            focus_roots: vec![1234],
            focus_reasons: vec!["game focus selected".to_owned()],
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
            last_action_at_unix_nanos: Vec::new(),
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
    fn idle_focus_blocks_new_experiment() {
        let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
        let state = ControllerRuntimeState::default();
        let mut observation = high_quality_observation(100);
        observation.focus_kind = Some(FocusGroupKind::Idle);
        observation.primary_situation = SituationKind::Idle;
        observation.focus_reasons = vec!["idle focus selected".to_owned()];
        let candidate = candidate_with_safety_class(SafetyClass::ReversibleLowRisk);

        let decision = decide_autotune_transition(&policy, &state, &observation, Some(candidate));

        match decision {
            AutotuneDecision::Noop { reason } => {
                assert!(reason.contains("idle or unknown"));
                assert!(reason.contains("observe-only"));
            }
            other => panic!("expected Noop for idle focus, got {other:?}"),
        }
    }

    #[test]
    fn compile_focus_blocks_gaming_cpu_isolation_profile() {
        let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
        let state = ControllerRuntimeState::default();
        let mut observation = high_quality_observation(100);
        observation.focus_kind = Some(FocusGroupKind::Compile);
        observation.primary_situation = SituationKind::CompileLoad;
        observation.focus_reasons = vec!["compile focus selected".to_owned()];
        let candidate = gaming_cpu_affinity_candidate();

        let decision = decide_autotune_transition(&policy, &state, &observation, Some(candidate));

        match decision {
            AutotuneDecision::Noop { reason } => {
                assert!(reason.contains("compile focus"));
                assert!(reason.contains("gaming CPU-isolation"));
            }
            other => panic!("expected Noop for compile focus gaming profile block, got {other:?}"),
        }
    }

    #[test]
    fn browser_focus_blocks_gaming_cpu_isolation_profile() {
        let policy = ControllerPolicy::for_mode(AutotuneMode::Suggest);
        let state = ControllerRuntimeState::default();
        let mut observation = high_quality_observation(100);
        observation.focus_kind = Some(FocusGroupKind::Browser);
        observation.primary_situation = SituationKind::BrowserFocused;
        observation.focus_reasons = vec!["browser focus selected".to_owned()];
        let candidate = gaming_cpu_affinity_candidate();

        let decision = decide_autotune_transition(&policy, &state, &observation, Some(candidate));

        match decision {
            AutotuneDecision::Noop { reason } => {
                assert!(reason.contains("browser focus"));
                assert!(reason.contains("desktop responsiveness"));
            }
            other => panic!("expected Noop for browser focus gaming profile block, got {other:?}"),
        }
    }

    #[test]
    fn critical_realtime_focus_warning_blocks_unsafe_candidate() {
        let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
        let state = ControllerRuntimeState::default();
        let mut observation = high_quality_observation(100);
        observation.focus_kind = Some(FocusGroupKind::Game);
        observation.focus_reasons = vec![
            "safety: critical realtime/input process present pid=55 comm='pipewire'; never lower or deprioritize this task".to_owned(),
        ];
        let candidate = candidate_with_safety_class(SafetyClass::ReversibleLowRisk);

        let decision = decide_autotune_transition(&policy, &state, &observation, Some(candidate));

        match decision {
            AutotuneDecision::Noop { reason } => {
                assert!(reason.contains("critical realtime/input"));
                assert!(reason.contains("blocked"));
            }
            other => panic!("expected Noop for critical realtime warning, got {other:?}"),
        }
    }

    #[test]
    fn game_focus_allows_existing_gaming_profile_path() {
        let policy = ControllerPolicy::for_mode(AutotuneMode::Suggest);
        let state = ControllerRuntimeState::default();
        let observation = high_quality_observation(100);
        let candidate = gaming_cpu_affinity_candidate();

        let decision = decide_autotune_transition(&policy, &state, &observation, Some(candidate));

        match decision {
            AutotuneDecision::Suggest { reason, .. } => {
                assert!(reason.contains("suggest mode reports candidate"));
            }
            other => panic!("expected Suggest for game focus, got {other:?}"),
        }
    }

    #[test]
    fn focus_policy_reverts_active_experiment_when_focus_becomes_idle() {
        let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
        let state = active_state_with_baseline_score(100);
        let mut observation = high_quality_observation(90);
        observation.focus_kind = Some(FocusGroupKind::Idle);
        observation.primary_situation = SituationKind::Idle;
        observation.focus_reasons = vec!["idle focus selected".to_owned()];

        let decision = decide_autotune_transition(&policy, &state, &observation, None);

        match decision {
            AutotuneDecision::Revert {
                experiment_id,
                reason,
            } => {
                assert_eq!(experiment_id, ExperimentId("experiment-1".to_owned()));
                assert!(reason.contains("focus policy blocks active experiment"));
                assert!(reason.contains("idle or unknown"));
            }
            other => panic!("expected Revert for active experiment with idle focus, got {other:?}"),
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
            last_action_at_unix_nanos: Vec::new(),
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

    #[test]
    fn policy_defaults_use_required_cooldown_values() {
        let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);

        assert_eq!(policy.cooldown_after_keep, Duration::from_secs(60));
        assert_eq!(policy.cooldown_after_revert, Duration::from_secs(120));
        assert_eq!(policy.cooldown_after_fault, Duration::from_secs(300));
        assert_eq!(
            policy.minimum_time_between_same_action,
            Duration::from_secs(300)
        );
    }

    #[test]
    fn same_action_hysteresis_blocks_repeated_candidate() {
        let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
        let observation = high_quality_observation(100);
        let candidate = candidate_with_safety_class(SafetyClass::ReversibleLowRisk);
        let mut state = ControllerRuntimeState::default();

        state.mark_candidate_action_attempted(&candidate, observation.now_unix_nanos);

        let decision = decide_autotune_transition(&policy, &state, &observation, Some(candidate));

        match decision {
            AutotuneDecision::EnterCooldown { duration, reason } => {
                assert_eq!(duration.as_secs(), 300);
                assert!(reason.contains("same action 'test' is still cooling down"));
                assert!(reason.contains("minimum_time_between_same_action is 300s"));
            }
            other => panic!("expected same-action EnterCooldown, got {other:?}"),
        }
    }

    #[test]
    fn same_action_hysteresis_allows_candidate_after_minimum_elapsed() {
        let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
        let mut observation = high_quality_observation(100);
        let candidate = candidate_with_safety_class(SafetyClass::ReversibleLowRisk);
        let mut state = ControllerRuntimeState::default();

        state.mark_candidate_action_attempted(&candidate, 1_000_000_000);
        observation.now_unix_nanos = 301_000_000_000;

        let decision = decide_autotune_transition(&policy, &state, &observation, Some(candidate));

        match decision {
            AutotuneDecision::StartExperiment { reason, .. } => {
                assert!(reason.contains("candidate passed data-quality and safety gates"));
            }
            other => panic!("expected StartExperiment after same-action cooldown, got {other:?}"),
        }
    }

    #[test]
    fn runtime_state_records_keep_revert_and_fault_cooldowns() {
        let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
        let now = 1_000_000_000;
        let mut state = ControllerRuntimeState::default();

        state.enter_cooldown_after_keep(&policy, now);
        assert_eq!(state.phase, ControllerPhase::Cooldown);
        assert_eq!(state.cooldown_until_unix_nanos, Some(61_000_000_000));

        state.enter_cooldown_after_revert(&policy, now);
        assert_eq!(state.phase, ControllerPhase::Cooldown);
        assert_eq!(state.cooldown_until_unix_nanos, Some(121_000_000_000));

        state.enter_cooldown_after_fault(&policy, now);
        assert_eq!(state.phase, ControllerPhase::Faulted);
        assert_eq!(state.cooldown_until_unix_nanos, Some(301_000_000_000));
    }

    #[test]
    fn faulted_controller_enters_fault_cooldown_then_reports_fault() {
        let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
        let mut observation = high_quality_observation(100);
        let mut state = ControllerRuntimeState::default();

        state.enter_cooldown_after_fault(&policy, 1_000_000_000);
        observation.now_unix_nanos = 2_000_000_000;

        let decision = decide_autotune_transition(&policy, &state, &observation, None);

        match decision {
            AutotuneDecision::EnterCooldown { duration, reason } => {
                assert_eq!(duration.as_secs(), 299);
                assert!(reason.contains("fault cooldown blocks repeated action"));
            }
            other => panic!("expected EnterCooldown during fault cooldown, got {other:?}"),
        }

        observation.now_unix_nanos = 301_000_000_000;
        let decision = decide_autotune_transition(&policy, &state, &observation, None);

        match decision {
            AutotuneDecision::Fault { reason } => {
                assert!(reason.contains("controller is faulted"));
            }
            other => panic!("expected Fault after fault cooldown expires, got {other:?}"),
        }
    }
}
