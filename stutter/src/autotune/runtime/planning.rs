//! Runtime-specific candidate planning helpers; this module owns simulated selection, not controller state transitions.

use crate::{
    actions::SafetyClass,
    autotune::{
        controller::ControllerRuntimeState,
        observation::AutotuneObservation,
        planner::{CandidateDenyReason, PlanResult},
        planning::{candidate::CandidateAction, dry_run::CandidateDryRunRecord},
        state::SituationKind,
    },
};

pub(crate) fn top_denied_reason_for_plan(plan: &PlanResult) -> Option<String> {
    plan.evaluations
        .iter()
        .find(|evaluation| !evaluation.eligible)
        .and_then(|evaluation| {
            evaluation
                .deny_reasons
                .first()
                .map(|reason| format!("{reason:?}"))
                .or_else(|| evaluation.deny_messages.first().cloned())
        })
        .or_else(|| plan.no_action_reason.clone())
}

pub(crate) fn plan_has_deny_reason(plan: &PlanResult, reason: CandidateDenyReason) -> bool {
    plan.evaluations
        .iter()
        .any(|evaluation| evaluation.deny_reasons.contains(&reason))
}

pub(crate) fn select_best_candidate_for_situation(
    candidates: &[CandidateAction],
    records: &[CandidateDryRunRecord],
    observation: &AutotuneObservation,
    max_safety_class: SafetyClass,
    state: &ControllerRuntimeState,
) -> Option<CandidateAction> {
    let mut ranked = candidates
        .iter()
        .filter_map(|candidate| {
            let record = records
                .iter()
                .find(|record| record.candidate_name == candidate.profile_name())?;

            if !record.eligible || record.safety_class > max_safety_class {
                return None;
            }

            if state
                .candidate_memory
                .cooldown_remaining_for_action(&candidate.action_id(), observation.now_unix_nanos)
                .is_some()
            {
                return None;
            }

            let rank = if matches!(candidate, CandidateAction::Fake { .. }) {
                0
            } else {
                candidate_situation_rank(candidate.profile_name(), observation.primary_situation)?
            };
            Some((rank, candidate.clone()))
        })
        .collect::<Vec<_>>();

    ranked.sort_by_key(|(rank, candidate)| (*rank, candidate.profile_name().to_owned()));
    ranked.into_iter().map(|(_, candidate)| candidate).next()
}

fn candidate_situation_rank(profile_name: &str, situation: SituationKind) -> Option<u8> {
    let name = profile_name.to_ascii_lowercase();

    match situation {
        SituationKind::GameCpuSchedulerPressure | SituationKind::GameFocused => {
            if name.contains("game-isolate-render") {
                Some(0)
            } else if name.contains("avoid-smt-contention") {
                Some(1)
            } else if name.contains("wine-server-dedicated") {
                Some(2)
            } else if name.contains("helper-spread") {
                Some(3)
            } else if name.contains("game") || name.contains("wine") || name.contains("helper") {
                Some(10)
            } else {
                None
            }
        }
        SituationKind::CompositorPressure => {
            if name.contains("game-compositor-separate") {
                Some(0)
            } else if name.contains("compositor") {
                Some(1)
            } else {
                None
            }
        }
        SituationKind::CpuPressure => {
            if name.contains("avoid-smt-contention") {
                Some(0)
            } else if name.contains("helper-spread") {
                Some(1)
            } else {
                None
            }
        }
        SituationKind::GameGpuBound
        | SituationKind::ThermalOrPowerLimit
        | SituationKind::IoPressure
        | SituationKind::IrqPressure
        | SituationKind::Idle
        | SituationKind::Unknown => None,
        SituationKind::CompileLoad
        | SituationKind::CompileCpuBound
        | SituationKind::CompileLinkerPressure
        | SituationKind::BrowserFocused
        | SituationKind::BrowserCpuPressure
        | SituationKind::BrowserGpuVideo
        | SituationKind::BrowserIoPressure
        | SituationKind::MediaPlayback
        | SituationKind::Recording
        | SituationKind::VirtualMachineLoad => None,
    }
}

pub(crate) fn simulated_dry_run_records(
    candidates: &[CandidateAction],
    active_target_count: usize,
) -> Vec<CandidateDryRunRecord> {
    candidates
        .iter()
        .map(|candidate| CandidateDryRunRecord {
            candidate_name: candidate.profile_name().to_owned(),
            affected_tasks: active_target_count.max(1),
            warnings: Vec::new(),
            safety_class: candidate.safety_class(),
            eligible: true,
            reason: None,
        })
        .collect()
}
