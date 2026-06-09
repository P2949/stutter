use std::{collections::BTreeMap, path::PathBuf};

use super::{
    model::{AutotuneStatus, StatusKeptAction, StatusTarget},
    render::{default_restore_path_display, format_last_decision, normalized_decision},
};
use crate::{
    actions::ActionId,
    autotune::history::{AutotuneHistoryEvent, AutotuneMode, ControllerPhase, TargetIdentity},
};

pub fn status_from_history_events(
    history_path: PathBuf,
    events: &[AutotuneHistoryEvent],
) -> AutotuneStatus {
    let Some(last) = events.last() else {
        return AutotuneStatus {
            phase: "Disabled".to_owned(),
            mode: "Observe".to_owned(),
            target: None,
            focus_group: None,
            current_score: None,
            active_profile: None,
            active_candidate: None,
            kept_actions: Vec::new(),
            last_decision: "no autotune history found".to_owned(),
            rollback_available: false,
            last_rollback_path: None,
            cooldown_remaining_seconds: None,
            data_quality: None,
            last_fault: None,
            manual_restore_command: "stutter autotune restore".to_owned(),
            planner: None,
            history_path,
        };
    };

    AutotuneStatus {
        phase: format_phase(last.phase),
        mode: format_mode(last.mode),
        target: last.target.as_ref().map(status_target_from_identity),
        focus_group: Some(format!("{:?}", last.situation)),
        current_score: Some(last.observation_summary.diagnostic_raw_score_total),
        active_profile: active_profile_from_events(events),
        active_candidate: active_candidate_from_events(events),
        kept_actions: kept_actions_from_events(events),
        last_decision: format_last_decision(last),
        rollback_available: rollback_available_from_events(events),
        last_rollback_path: last_rollback_path_from_events(events),
        cooldown_remaining_seconds: cooldown_remaining_seconds_from_events(events),
        data_quality: Some(last.observation_summary.data_quality.clone()),
        last_fault: last_fault_from_events(events),
        manual_restore_command: "stutter autotune restore".to_owned(),
        planner: last.planner.clone(),
        history_path,
    }
}
pub(crate) fn cooldown_remaining_seconds_from_unix_nanos(cooldown_until: u128) -> u64 {
    let now = crate::audit::unix_nanos_now();

    if now >= cooldown_until {
        return 0;
    }

    let remaining_nanos = cooldown_until.saturating_sub(now);
    let remaining_seconds = remaining_nanos.div_ceil(1_000_000_000);
    remaining_seconds.min(u64::MAX as u128) as u64
}

fn cooldown_remaining_seconds_from_events(events: &[AutotuneHistoryEvent]) -> Option<u64> {
    for event in events.iter().rev() {
        let Some(cooldown_until) =
            cooldown_until_unix_nanos_from_policy(&event.decision.rollback_policy)
        else {
            continue;
        };

        return Some(cooldown_remaining_seconds_from_unix_nanos(cooldown_until));
    }

    None
}

fn cooldown_until_unix_nanos_from_policy(policy: &str) -> Option<u128> {
    for part in policy.split(';') {
        let trimmed = part.trim();
        let Some(value) = trimmed.strip_prefix("cooldown_until_unix_nanos=") else {
            continue;
        };

        if let Ok(parsed) = value.parse::<u128>() {
            return Some(parsed);
        }
    }

    None
}

fn status_target_from_identity(target: &TargetIdentity) -> StatusTarget {
    StatusTarget {
        comm: target.process_comm.clone(),
        pid: target.root_pid,
    }
}

fn active_profile_from_events(events: &[AutotuneHistoryEvent]) -> Option<String> {
    for event in events.iter().rev() {
        let decision = normalized_decision(&event.decision.decision);

        if event.rollback_performed || decision == "candidate_reverted" || decision == "restored" {
            return None;
        }

        if decision == "candidate_kept"
            && let Some(candidate_name) = event.decision.candidate_name.as_ref()
        {
            return Some(candidate_name.clone());
        }
    }

    None
}

fn active_candidate_from_events(events: &[AutotuneHistoryEvent]) -> Option<String> {
    for event in events.iter().rev() {
        let decision = normalized_decision(&event.decision.decision);

        if matches!(
            decision.as_str(),
            "candidate_reverted" | "candidate_kept" | "restored" | "cooldown_entered"
        ) || event.rollback_performed
        {
            return None;
        }

        if matches!(
            decision.as_str(),
            "candidate_started" | "candidate_applied" | "suggested"
        ) && let Some(candidate_name) = event.decision.candidate_name.as_ref()
        {
            return Some(candidate_name.clone());
        }
    }

    None
}

fn kept_actions_from_events(events: &[AutotuneHistoryEvent]) -> Vec<StatusKeptAction> {
    let mut kept = BTreeMap::<ActionId, StatusKeptAction>::new();

    for event in events {
        let decision = normalized_decision(&event.decision.decision);

        if decision == "restored" {
            kept.clear();
            continue;
        }

        if event.rollback_performed || decision == "candidate_reverted" {
            if let Some(action_id) = event.action_id.as_ref() {
                kept.remove(action_id);
            } else {
                kept.clear();
            }
            continue;
        }

        if decision == "candidate_kept" {
            let action_id = event.action_id.clone().unwrap_or_else(|| {
                ActionId::new(
                    event
                        .decision
                        .candidate_name
                        .clone()
                        .unwrap_or_else(|| "unknown".to_owned()),
                )
            });
            kept.insert(
                action_id.clone(),
                StatusKeptAction {
                    candidate_name: event
                        .decision
                        .candidate_name
                        .clone()
                        .unwrap_or_else(|| "unknown".to_owned()),
                    action_id,
                    action_kind: event
                        .decision
                        .action_kind
                        .clone()
                        .unwrap_or_else(|| "unknown".to_owned()),
                    safety_class: event
                        .decision
                        .safety_class
                        .as_ref()
                        .map(|safety_class| format!("{safety_class:?}"))
                        .unwrap_or_else(|| "unknown".to_owned()),
                    kept_unix_nanos: event.unix_nanos,
                },
            );
        }
    }

    kept.into_values().collect()
}

fn rollback_available_from_events(events: &[AutotuneHistoryEvent]) -> bool {
    for event in events.iter().rev() {
        let decision = normalized_decision(&event.decision.decision);

        if event.rollback_performed || decision == "candidate_reverted" || decision == "restored" {
            return false;
        }

        if event.action_id.is_some()
            && matches!(
                decision.as_str(),
                "candidate_applied" | "candidate_kept" | "cooldown_entered"
            )
            && event.decision.rollback_policy.contains("rollback")
        {
            return true;
        }
    }

    false
}

fn last_rollback_path_from_events(events: &[AutotuneHistoryEvent]) -> Option<String> {
    for event in events.iter().rev() {
        let decision = normalized_decision(&event.decision.decision);

        if event.rollback_performed || decision == "candidate_reverted" || decision == "restored" {
            return None;
        }

        if event.decision.rollback_policy.contains("rollback") {
            return Some(default_restore_path_display());
        }
    }

    None
}

fn last_fault_from_events(events: &[AutotuneHistoryEvent]) -> Option<String> {
    for event in events.iter().rev() {
        let decision = normalized_decision(&event.decision.decision);

        if decision == "restored" {
            return None;
        }

        if matches!(event.phase, ControllerPhase::Faulted) || decision == "faulted" {
            return Some(event.reason.clone().clone());
        }
    }

    None
}
pub(crate) fn improvement_percent(event: &AutotuneHistoryEvent) -> Option<f64> {
    let before = event.score_before.as_ref()?.score.total;
    let after = event.score_after.as_ref()?.score.total;

    if before == 0 || after >= before {
        return None;
    }

    Some(((before - after) as f64 / before as f64) * 100.0)
}
fn format_phase(phase: ControllerPhase) -> String {
    format!("{phase:?}")
}

fn format_mode(mode: AutotuneMode) -> String {
    format!("{mode:?}")
}
