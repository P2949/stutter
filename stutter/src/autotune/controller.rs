use std::time::Duration;

use super::{
    candidate_memory::{
        CandidateContextHashInput, CandidateMemory, CandidateMemoryRecord, CandidateMemoryResult,
        CandidateResultRecordInput,
    },
    comparison::{
        DEFAULT_SCORE_COMPARISON_CONFIG, ExperimentResult, ScoreComparisonConfig,
        compare_scores_with_config,
    },
    decision::{AutotuneDecision, CandidateAction, ExperimentId},
    experiment::WindowScore,
    observation::AutotuneObservation,
    quality::OnlineDataQuality,
    state::{AutotuneMode, ControllerPhase},
};
use crate::{
    actions::SafetyClass,
    daemon::{
        DaemonConfig, DaemonPolicy,
        policy::{ActionSource, DaemonMode, DaemonPolicyBuildInput, build_daemon_policy},
    },
    focus::FocusGroupKind,
    process_tree::TaskClass,
};

pub(crate) const DEFAULT_MIN_FOCUS_CONFIDENCE: f32 = super::DEFAULT_MIN_FOCUS_CONFIDENCE;

#[derive(Clone, Debug)]
pub struct ControllerPolicy {
    pub mode: DaemonMode,
    pub max_safety_class: SafetyClass,
    pub min_improvement_percent: f64,
    pub max_regression_percent: f64,
    pub min_focus_confidence: f32,
    pub cooldown_after_keep: Duration,
    pub cooldown_after_revert: Duration,
    pub cooldown_after_fault: Duration,
    pub minimum_time_between_same_action: Duration,
}

impl ControllerPolicy {
    pub fn from_daemon_policy(policy: &DaemonPolicy) -> Self {
        Self {
            mode: policy.mode,
            max_safety_class: policy.max_safety_class.clone(),
            min_improvement_percent: DEFAULT_SCORE_COMPARISON_CONFIG.min_improvement_percent,
            max_regression_percent: DEFAULT_SCORE_COMPARISON_CONFIG.max_regression_percent,
            min_focus_confidence: policy.min_confidence,
            cooldown_after_keep: Duration::from_secs(60),
            cooldown_after_revert: Duration::from_secs(120),
            cooldown_after_fault: Duration::from_secs(300),
            minimum_time_between_same_action: Duration::from_secs(300),
        }
    }

    pub fn for_mode(mode: AutotuneMode) -> Self {
        let daemon_mode = daemon_mode_from_autotune_mode(mode);
        let mut config = DaemonConfig {
            mode: daemon_mode,
            source: ActionSource::AutotuneRuntime,
            ..DaemonConfig::default()
        };
        config.safety.min_confidence = DEFAULT_MIN_FOCUS_CONFIDENCE;
        let policy = build_daemon_policy(DaemonPolicyBuildInput {
            config: &config,
            remote_context: None,
        });

        Self::from_daemon_policy(&policy)
    }

    pub fn can_start_experiment(&self) -> bool {
        self.mode.supports_apply()
    }
}

fn daemon_mode_from_autotune_mode(mode: AutotuneMode) -> DaemonMode {
    match mode {
        AutotuneMode::Observe => DaemonMode::Observe,
        AutotuneMode::Suggest => DaemonMode::Suggest,
        AutotuneMode::ApplyLowRisk => DaemonMode::ApplyLowRisk,
        AutotuneMode::ApplyMediumRisk => DaemonMode::ApplyMediumRisk,
        AutotuneMode::ApplyHighRisk => DaemonMode::ApplyHighRisk,
    }
}

#[derive(Clone, Debug)]
pub struct ActiveExperiment {
    pub experiment_id: ExperimentId,
    pub candidate: CandidateAction,
    pub baseline_score: WindowScore,
}

#[derive(Clone, Debug)]
pub struct ControllerRuntimeState {
    pub phase: ControllerPhase,
    pub active_experiment: Option<ActiveExperiment>,
    pub cooldown_until_unix_nanos: Option<u128>,
    pub candidate_memory: CandidateMemory,
}

impl Default for ControllerRuntimeState {
    fn default() -> Self {
        Self {
            phase: ControllerPhase::Observing,
            active_experiment: None,
            cooldown_until_unix_nanos: None,
            candidate_memory: CandidateMemory::default(),
        }
    }
}

pub struct ControllerCandidateResultInput<'a> {
    pub candidate: &'a CandidateAction,
    pub observation: &'a AutotuneObservation,
    pub cpu_topology_signature: Option<&'a str>,
    pub result: CandidateMemoryResult,
    pub diagnostic_baseline_raw_score_total: Option<u64>,
    pub diagnostic_current_raw_score_total: Option<u64>,
    pub rollback_reason: Option<String>,
    pub cooldown_expires_unix_nanos: Option<u128>,
}

impl ControllerRuntimeState {
    pub fn mark_candidate_action_attempted(
        &mut self,
        candidate: &CandidateAction,
        now_unix_nanos: u128,
    ) {
        let context = CandidateContextHashInput::for_candidate(candidate);
        self.candidate_memory
            .record_attempt(candidate, &context, now_unix_nanos, None);
    }

    pub fn record_candidate_attempt(
        &mut self,
        candidate: &CandidateAction,
        observation: &AutotuneObservation,
        cpu_topology_signature: Option<&str>,
        cooldown_expires_unix_nanos: Option<u128>,
    ) -> CandidateMemoryRecord {
        let context = CandidateContextHashInput::from_observation(
            candidate,
            observation,
            cpu_topology_signature,
        );

        self.candidate_memory.record_attempt(
            candidate,
            &context,
            observation.now_unix_nanos,
            cooldown_expires_unix_nanos,
        )
    }

    pub fn record_candidate_result(
        &mut self,
        input: ControllerCandidateResultInput<'_>,
    ) -> CandidateMemoryRecord {
        let context = CandidateContextHashInput::from_observation(
            input.candidate,
            input.observation,
            input.cpu_topology_signature,
        );

        self.candidate_memory
            .record_result(CandidateResultRecordInput {
                candidate: input.candidate,
                context: &context,
                now_unix_nanos: input.observation.now_unix_nanos,
                result: input.result,
                diagnostic_baseline_raw_score_total: input.diagnostic_baseline_raw_score_total,
                diagnostic_current_raw_score_total: input.diagnostic_current_raw_score_total,
                rollback_reason: input.rollback_reason,
                cooldown_expires_unix_nanos: input.cooldown_expires_unix_nanos,
            })
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

        let comparison = compare_scores_with_config(
            crate::autotune::comparison::ScoreComparisonInput {
                baseline: &active.baseline_score,
                candidate: &observation.to_window_score(),
                data_quality: crate::autotune::comparison::ExperimentDataQuality::High,
                target_disappeared: false,
            },
            &ScoreComparisonConfig {
                min_improvement_percent: policy.min_improvement_percent,
                max_regression_percent: policy.max_regression_percent,
                ..DEFAULT_SCORE_COMPARISON_CONFIG
            },
            None,
        );

        match comparison {
            ExperimentResult::Regressed { regression_percent } => {
                return AutotuneDecision::Revert {
                    experiment_id: active.experiment_id.clone(),
                    reason: format!(
                        "candidate regressed normalized score by {:.1}% which exceeds max_regression_percent {:.1}%",
                        regression_percent, policy.max_regression_percent
                    ),
                };
            }
            ExperimentResult::Improved {
                improvement_percent,
            } => {
                return AutotuneDecision::KeepCurrent {
                    experiment_id: active.experiment_id.clone(),
                    reason: format!(
                        "candidate improved normalized score by {:.1}% which meets min_improvement_percent {:.1}%",
                        improvement_percent, policy.min_improvement_percent
                    ),
                };
            }
            ExperimentResult::Inconclusive { reason } => {
                return AutotuneDecision::Noop {
                    reason: format!("active experiment is still inconclusive: {}", reason),
                };
            }
            ExperimentResult::Invalid { reason } => {
                return AutotuneDecision::Revert {
                    experiment_id: active.experiment_id.clone(),
                    reason: format!("invalid active experiment score comparison: {}", reason),
                };
            }
        }
    }

    if let Some(decision) =
        cooldown_block_decision(state, observation, "cooldown blocks repeated action")
    {
        return decision;
    }

    if policy.mode == DaemonMode::Observe {
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
                candidate.action_id().as_str(),
                policy.minimum_time_between_same_action.as_secs()
            ),
        };
    }

    if policy.mode == DaemonMode::Suggest {
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
    let action_id = candidate.action_id();

    if let Some(duration) = state
        .candidate_memory
        .cooldown_remaining_for_action(&action_id, observation.now_unix_nanos)
    {
        return Some(duration);
    }

    if policy.minimum_time_between_same_action.is_zero() {
        return None;
    }

    let record = state.candidate_memory.last_for_action(&action_id)?;

    let next_allowed = record
        .last_tried_unix_nanos
        .saturating_add(policy.minimum_time_between_same_action.as_nanos());
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
        CandidateAction::CpuAffinityProfile { plan } => {
            let lower_name = plan.profile_name.to_ascii_lowercase();
            lower_name.contains("game")
                || lower_name.contains("gaming")
                || lower_name.contains("isolation")
                || plan.profile.rules.iter().any(|rule| {
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
        CandidateAction::Fake { .. }
        | CandidateAction::Nice { .. }
        | CandidateAction::IoPrio { .. }
        | CandidateAction::Uclamp { .. }
        | CandidateAction::CgroupPlacement { .. }
        | CandidateAction::IrqAffinity { .. }
        | CandidateAction::CpuPower { .. }
        | CandidateAction::GpuPower { .. }
        | CandidateAction::VmKnob { .. } => false,
    }
}

#[cfg(test)]
mod tests;
