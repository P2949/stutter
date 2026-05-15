use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use super::history::{
    AutotuneHistoryEvent, AutotuneMode, ControllerPhase, TargetIdentity,
    default_autotune_history_path, read_autotune_history_events,
};
use crate::daemon::{
    DaemonMode, DaemonPhase, DaemonState, DaemonTargetState, default_daemon_state_snapshot_path,
    load_daemon_state,
};

#[derive(Clone, Debug)]
pub struct AutotuneStatusCommandInput {
    pub json: bool,
    pub history_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutotuneStatus {
    pub phase: String,
    pub mode: String,
    pub target: Option<StatusTarget>,
    pub focus_group: Option<String>,
    pub current_score: Option<u64>,
    pub active_profile: Option<String>,
    pub active_candidate: Option<String>,
    pub last_decision: String,
    pub rollback_available: bool,
    pub last_rollback_path: Option<String>,
    pub cooldown_remaining_seconds: Option<u64>,
    pub data_quality: Option<String>,
    pub last_fault: Option<String>,
    pub manual_restore_command: String,
    pub history_path: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatusTarget {
    pub comm: String,
    pub pid: u32,
}

pub fn autotune_status_command(input: AutotuneStatusCommandInput) -> anyhow::Result<()> {
    let history_path = input
        .history_path
        .unwrap_or_else(default_autotune_history_path);
    let status = load_autotune_status(&history_path)?;

    if input.json {
        println!("{}", serde_json::to_string_pretty(&status)?);
    } else {
        print!("{}", render_autotune_status_text(&status));
    }

    Ok(())
}

pub fn load_autotune_status(history_path: &Path) -> anyhow::Result<AutotuneStatus> {
    let daemon_state_path = daemon_state_path_for_history_path(history_path);
    if daemon_state_path.exists() {
        let state = load_daemon_state(&daemon_state_path).with_context(|| {
            format!(
                "failed to load daemon state snapshot {}",
                daemon_state_path.display()
            )
        })?;
        return Ok(status_from_daemon_state(daemon_state_path, &state));
    }

    let events = read_autotune_history_events(history_path)?;
    Ok(status_from_history_events(
        history_path.to_path_buf(),
        &events,
    ))
}

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
            last_decision: "no autotune history found".to_owned(),
            rollback_available: false,
            last_rollback_path: None,
            cooldown_remaining_seconds: None,
            data_quality: None,
            last_fault: None,
            manual_restore_command: "stutter autotune restore".to_owned(),
            history_path,
        };
    };

    AutotuneStatus {
        phase: format_phase(last.phase),
        mode: format_mode(last.mode),
        target: last.target.as_ref().map(status_target_from_identity),
        focus_group: Some(format!("{:?}", last.situation)),
        current_score: Some(last.observation_summary.score_total),
        active_profile: active_profile_from_events(events),
        active_candidate: active_candidate_from_events(events),
        last_decision: format_last_decision(last),
        rollback_available: rollback_available_from_events(events),
        last_rollback_path: last_rollback_path_from_events(events),
        cooldown_remaining_seconds: cooldown_remaining_seconds_from_events(events),
        data_quality: Some(last.observation_summary.data_quality.clone()),
        last_fault: last_fault_from_events(events),
        manual_restore_command: "stutter autotune restore".to_owned(),
        history_path,
    }
}

pub fn status_from_daemon_state(path: PathBuf, state: &DaemonState) -> AutotuneStatus {
    AutotuneStatus {
        phase: format_daemon_phase(state.phase),
        mode: format_daemon_mode(state.mode),
        target: state
            .active_target
            .as_ref()
            .and_then(status_target_from_daemon_target),
        focus_group: None,
        current_score: state
            .last_decision
            .as_ref()
            .and_then(|decision| decision.score_total),
        active_profile: active_profile_from_daemon_state(state),
        active_candidate: active_candidate_from_daemon_state(state),
        last_decision: last_decision_from_daemon_state(state),
        rollback_available: state
            .active_rollback
            .as_ref()
            .map(|rollback| rollback.rollback_available)
            .unwrap_or(false),
        last_rollback_path: last_rollback_path_from_daemon_state(state),
        cooldown_remaining_seconds: state
            .cooldown_until_unix_nanos
            .map(cooldown_remaining_seconds_from_unix_nanos),
        data_quality: data_quality_from_daemon_state(state),
        last_fault: state.faulted.as_ref().map(|fault| fault.reason.clone()),
        manual_restore_command: manual_restore_command_from_daemon_state(state),
        history_path: path,
    }
}

pub fn render_autotune_status_text(status: &AutotuneStatus) -> String {
    let target = match &status.target {
        Some(target) => format!("{} pid={}", target.comm, target.pid),
        None => "none".to_owned(),
    };

    format!(
        "phase: {}\nmode: {}\ntarget: {}\nfocus_group: {}\ncurrent_score: {}\nactive_profile: {}\nactive_candidate: {}\nlast_decision: {}\nrollback_available: {}\nlast_rollback_path: {}\ncooldown_remaining_seconds: {}\ndata_quality: {}\nlast_fault: {}\nmanual_restore_command: {}\n",
        status.phase,
        status.mode,
        target,
        status.focus_group.as_deref().unwrap_or("none"),
        status
            .current_score
            .map(|score| score.to_string())
            .unwrap_or_else(|| "none".to_owned()),
        status.active_profile.as_deref().unwrap_or("none"),
        status.active_candidate.as_deref().unwrap_or("none"),
        status.last_decision,
        if status.rollback_available {
            "yes"
        } else {
            "no"
        },
        status.last_rollback_path.as_deref().unwrap_or("none"),
        status
            .cooldown_remaining_seconds
            .map(|seconds| seconds.to_string())
            .unwrap_or_else(|| "none".to_owned()),
        status.data_quality.as_deref().unwrap_or("none"),
        status.last_fault.as_deref().unwrap_or("none"),
        status.manual_restore_command
    )
}

fn daemon_state_path_for_history_path(history_path: &Path) -> PathBuf {
    history_path
        .parent()
        .map(|parent| parent.join("daemon_state.json"))
        .unwrap_or_else(default_daemon_state_snapshot_path)
}

fn format_daemon_phase(phase: DaemonPhase) -> String {
    phase.lifecycle_label().to_owned()
}

fn format_daemon_mode(mode: DaemonMode) -> String {
    format!("{mode:?}")
}

fn status_target_from_daemon_target(target: &DaemonTargetState) -> Option<StatusTarget> {
    let pid = target.root_pid?;
    let comm = target.comm.clone().unwrap_or_else(|| format!("pid-{pid}"));

    Some(StatusTarget { comm, pid })
}

fn active_profile_from_daemon_state(state: &DaemonState) -> Option<String> {
    if matches!(state.phase, DaemonPhase::Keep | DaemonPhase::Cooldown) {
        return state
            .active_experiment
            .as_ref()
            .and_then(|experiment| experiment.candidate_name.clone());
    }

    None
}

fn active_candidate_from_daemon_state(state: &DaemonState) -> Option<String> {
    if matches!(
        state.phase,
        DaemonPhase::Decide | DaemonPhase::Apply | DaemonPhase::Measure
    ) {
        return state
            .active_experiment
            .as_ref()
            .and_then(|experiment| experiment.candidate_name.clone());
    }

    None
}

fn last_decision_from_daemon_state(state: &DaemonState) -> String {
    if let Some(decision) = state.last_decision.as_ref() {
        let normalized = normalized_decision(&decision.decision);
        if decision.reason.trim().is_empty() {
            return normalized;
        }
        return format!("{normalized}: {}", decision.reason);
    }

    if let Some(fault) = state.faulted.as_ref() {
        return format!("faulted: {}", fault.reason);
    }

    "daemon_state_snapshot_loaded".to_owned()
}

fn last_rollback_path_from_daemon_state(state: &DaemonState) -> Option<String> {
    state
        .active_rollback
        .as_ref()
        .and_then(|rollback| rollback.token.as_ref())
        .and_then(|token| token.restore_path())
        .map(|path| restore_path_display(path))
}

fn data_quality_from_daemon_state(state: &DaemonState) -> Option<String> {
    state
        .degraded
        .iter()
        .find(|status| status.category == "data_quality")
        .map(|status| status.message.clone())
}

fn manual_restore_command_from_daemon_state(state: &DaemonState) -> String {
    state
        .faulted
        .as_ref()
        .and_then(|fault| fault.manual_restore_command.clone())
        .or_else(|| {
            state
                .active_rollback
                .as_ref()
                .and_then(|rollback| rollback.manual_restore_command.clone())
        })
        .unwrap_or_else(|| "stutter autotune restore".to_owned())
}

fn cooldown_remaining_seconds_from_unix_nanos(cooldown_until: u128) -> u64 {
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
            return Some(event.reason.clone());
        }
    }

    None
}

fn default_restore_path_display() -> String {
    let path = crate::affinity::default_restore_path();
    restore_path_display(&path)
}

fn restore_path_display(path: &Path) -> String {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return path.display().to_string();
    };

    match path.strip_prefix(&home) {
        Ok(stripped) => format!("~/{}", stripped.display()),
        Err(_) => path.display().to_string(),
    }
}

fn format_last_decision(event: &AutotuneHistoryEvent) -> String {
    let decision = normalized_decision(&event.decision.decision);

    if let Some(improvement) = improvement_percent(event)
        && decision == "candidate_kept"
    {
        return format!("candidate_kept, improvement={:.1}%", improvement);
    }

    if event.rollback_performed {
        return format!("{decision}, rollback performed");
    }

    if !event.reason.trim().is_empty() {
        return format!("{decision}: {}", event.reason);
    }

    decision
}

fn normalized_decision(decision: &str) -> String {
    match decision {
        "Noop" | "noop" | "observed" => "observed".to_owned(),
        "Suggest" | "suggest" | "suggested" => "suggested".to_owned(),
        "StartExperiment" | "candidate_started" => "candidate_started".to_owned(),
        "candidate_applied" => "candidate_applied".to_owned(),
        "KeepCurrent" | "Kept" | "Keep" | "Improved" | "candidate_kept" => {
            "candidate_kept".to_owned()
        }
        "Revert" | "Reverted" | "candidate_reverted" => "candidate_reverted".to_owned(),
        "EnterCooldown" | "Cooldown" | "cooldown" | "cooldown_entered" => {
            "cooldown_entered".to_owned()
        }
        "Fault" | "Faulted" | "EmergencyRestoreFault" | "CrashRecoveryFault" | "faulted" => {
            "faulted".to_owned()
        }
        "EmergencyRestore" | "CrashRecoveryRollback" | "restored" => "restored".to_owned(),
        other => humanize_decision(other).replace(' ', "_"),
    }
}

fn improvement_percent(event: &AutotuneHistoryEvent) -> Option<f64> {
    let before = event.score_before.as_ref()?.score.total;
    let after = event.score_after.as_ref()?.score.total;

    if before == 0 || after >= before {
        return None;
    }

    Some(((before - after) as f64 / before as f64) * 100.0)
}

fn humanize_decision(decision: &str) -> String {
    let mut out = String::new();

    for (idx, ch) in decision.chars().enumerate() {
        if idx > 0 && ch.is_uppercase() {
            out.push(' ');
        }
        out.push(ch.to_ascii_lowercase());
    }

    out
}

fn format_phase(phase: ControllerPhase) -> String {
    format!("{phase:?}")
}

fn format_mode(mode: AutotuneMode) -> String {
    format!("{mode:?}")
}

#[cfg(test)]
mod tests {
    use std::{fs, io::Write};

    use super::*;
    use crate::{
        actions::{RollbackToken, SafetyClass},
        autotune::history::{AutotuneDecisionSummary, ObservationSummary, SituationKind},
        daemon::{
            DaemonDecisionState, DaemonDegradedStatus, DaemonExperimentState, DaemonFaultState,
            DaemonPhase, DaemonRollbackState, DaemonState, DaemonStateSnapshotWriter,
            DaemonTargetState,
        },
        scorer::StutterScore,
    };

    fn temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-autotune-status-test-{name}-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn score(total: u64) -> crate::autotune::experiment::WindowScore {
        crate::autotune::experiment::WindowScore {
            started_unix_nanos: 100,
            finished_unix_nanos: 200,
            interval_count: 10,
            scored_samples: 100,
            scored_task_count: 2,
            score: StutterScore {
                total,
                ..StutterScore::default()
            },
        }
    }

    fn observation() -> ObservationSummary {
        ObservationSummary {
            target_present: true,
            active_target_count: 31,
            scored_task_count: 2,
            interval_count: 10,
            scored_samples: 100,
            score_total: 818,
            over_1ms: 0,
            over_2ms: 0,
            over_5ms: 0,
            frame_p99_ms: 12.0,
            frame_max_ms: 20.0,
            drop_counter_total: 0,
            data_quality: "High".to_owned(),
        }
    }

    fn target() -> TargetIdentity {
        TargetIdentity {
            root_pid: 1234,
            process_comm: "KingdomCome.exe".to_owned(),
            process_starttime_ticks: Some(99),
            exe_dev: Some(1),
            exe_ino: Some(2),
            active_task_count: 31,
        }
    }

    fn kept_event() -> AutotuneHistoryEvent {
        AutotuneHistoryEvent {
            schema_version: 1,
            unix_nanos: 1,
            controller_id: "controller-1".to_owned(),
            phase: ControllerPhase::Cooldown,
            mode: AutotuneMode::ApplyLowRisk,
            target: Some(target()),
            situation: SituationKind::GameCpuSchedulerPressure,
            observation_summary: observation(),
            decision: AutotuneDecisionSummary {
                decision: "KeepCurrent".to_owned(),
                candidate_name: Some("game-main-suggested".to_owned()),
                action_kind: Some("cpu_affinity_profile".to_owned()),
                eligible: true,
                rollback_policy: "rollback-on-exit".to_owned(),
            },
            experiment_id: Some("experiment-1".to_owned()),
            action_id: Some("cpu-affinity-profile:game-main-suggested".to_owned()),
            score_before: Some(score(1_000)),
            score_after: Some(score(818)),
            rollback_performed: false,
            reason: "candidate improved by 18.20%; kept as current active profile".to_owned(),
        }
    }

    #[test]
    fn status_from_history_matches_text_example() {
        let status =
            status_from_history_events(PathBuf::from("/tmp/history.jsonl"), &[kept_event()]);

        assert_eq!(status.phase, "Cooldown");
        assert_eq!(status.mode, "ApplyLowRisk");
        assert_eq!(
            status.target,
            Some(StatusTarget {
                comm: "KingdomCome.exe".to_owned(),
                pid: 1234,
            })
        );
        assert_eq!(
            status.active_profile.as_deref(),
            Some("game-main-suggested")
        );
        assert_eq!(status.last_decision, "candidate_kept, improvement=18.2%");
        assert!(status.rollback_available);
        assert!(status.last_rollback_path.is_some());

        let rendered = render_autotune_status_text(&status);
        assert!(rendered.contains("phase: Cooldown"));
        assert!(rendered.contains("mode: ApplyLowRisk"));
        assert!(rendered.contains("target: KingdomCome.exe pid=1234"));
        assert!(rendered.contains("active_profile: game-main-suggested"));
        assert!(rendered.contains("last_decision: candidate_kept, improvement=18.2%"));
        assert!(rendered.contains("rollback_available: yes"));
        assert!(rendered.contains("last_rollback_path: "));
    }

    #[test]
    fn json_status_serializes() {
        let status =
            status_from_history_events(PathBuf::from("/tmp/history.jsonl"), &[kept_event()]);

        let json = serde_json::to_string_pretty(&status).unwrap();

        assert!(json.contains("\"phase\": \"Cooldown\""));
        assert!(json.contains("\"mode\": \"ApplyLowRisk\""));
        assert!(json.contains("\"active_profile\": \"game-main-suggested\""));
        assert!(json.contains("\"rollback_available\": true"));
    }

    #[test]
    fn empty_history_reports_disabled_status() {
        let status = status_from_history_events(PathBuf::from("/tmp/history.jsonl"), &[]);

        assert_eq!(status.phase, "Disabled");
        assert_eq!(status.mode, "Observe");
        assert_eq!(status.target, None);
        assert_eq!(status.active_profile, None);
        assert_eq!(status.last_decision, "no autotune history found");
        assert!(!status.rollback_available);
        assert_eq!(status.last_rollback_path, None);
    }

    #[test]
    fn rollback_event_clears_active_profile_and_rollback_available() {
        let mut rolled_back = kept_event();
        rolled_back.unix_nanos = 2;
        rolled_back.decision.decision = "Revert".to_owned();
        rolled_back.rollback_performed = true;
        rolled_back.reason = "regressed; rollback performed".to_owned();

        let status = status_from_history_events(
            PathBuf::from("/tmp/history.jsonl"),
            &[kept_event(), rolled_back],
        );

        assert_eq!(status.active_profile, None);
        assert!(!status.rollback_available);
        assert_eq!(status.last_rollback_path, None);
        assert_eq!(
            status.last_decision,
            "candidate_reverted, rollback performed"
        );
    }

    #[test]
    fn status_reports_cooldown_remaining_from_history_policy_metadata() {
        let mut event = kept_event();
        event.decision.decision = "cooldown_entered".to_owned();
        event.decision.rollback_policy = format!(
            "rollback-on-restore;cooldown_until_unix_nanos={};manual_restore_command=stutter_autotune_restore",
            crate::audit::unix_nanos_now().saturating_add(60_000_000_000)
        );

        let status = status_from_history_events(PathBuf::from("/tmp/history.jsonl"), &[event]);

        assert!(status.cooldown_remaining_seconds.unwrap_or(0) > 0);
        assert!(status.cooldown_remaining_seconds.unwrap_or(0) <= 60);
    }

    #[test]
    fn restored_event_clears_rollback_active_profile_and_last_fault() {
        let mut faulted = kept_event();
        faulted.unix_nanos = 2;
        faulted.phase = ControllerPhase::Faulted;
        faulted.decision.decision = "faulted".to_owned();
        faulted.reason = "rollback failed".to_owned();

        let mut restored = kept_event();
        restored.unix_nanos = 3;
        restored.decision.decision = "restored".to_owned();
        restored.rollback_performed = true;
        restored.reason = "manual restore succeeded".to_owned();

        let status = status_from_history_events(
            PathBuf::from("/tmp/history.jsonl"),
            &[kept_event(), faulted, restored],
        );

        assert_eq!(status.active_profile, None);
        assert_eq!(status.active_candidate, None);
        assert!(!status.rollback_available);
        assert_eq!(status.last_rollback_path, None);
        assert_eq!(status.last_fault, None);
        assert_eq!(status.last_decision, "restored, rollback performed");
    }

    #[test]
    fn candidate_applied_event_sets_active_candidate_and_rollback_available() {
        let mut event = kept_event();
        event.phase = ControllerPhase::Measuring;
        event.decision.decision = "candidate_applied".to_owned();
        event.decision.rollback_policy =
            "rollback-on-restore;manual_restore_command=stutter_autotune_restore".to_owned();

        let status = status_from_history_events(PathBuf::from("/tmp/history.jsonl"), &[event]);

        assert_eq!(
            status.active_candidate.as_deref(),
            Some("game-main-suggested")
        );
        assert!(status.rollback_available);
        assert!(status.last_rollback_path.is_some());
    }

    #[test]
    fn status_from_daemon_state_reports_snapshot_fields() {
        let state = DaemonState {
            mode: DaemonMode::ApplyLowRisk,
            phase: DaemonPhase::Measure,
            cooldown_until_unix_nanos: Some(
                crate::audit::unix_nanos_now().saturating_add(30_000_000_000),
            ),
            active_target: Some(DaemonTargetState {
                root_pid: Some(1234),
                active_targets: 2,
                comm: Some("KingdomCome.exe".to_owned()),
            }),
            active_experiment: Some(DaemonExperimentState {
                experiment_id: "experiment-1".to_owned(),
                action_id: "cpu-affinity-profile:game-main".to_owned(),
                candidate_name: Some("game-main".to_owned()),
                safety_class: SafetyClass::ReversibleLowRisk,
                started_unix_nanos: Some(100),
            }),
            active_rollback: Some(DaemonRollbackState {
                action_id: "cpu-affinity-profile:game-main".to_owned(),
                rollback_available: true,
                token: Some(RollbackToken::CpuAffinityRestoreFile {
                    path: PathBuf::from("/tmp/stutter-restore.json"),
                    affected_tasks: 31,
                }),
                manual_restore_command: Some("stutter restore".to_owned()),
            }),
            last_decision: Some(DaemonDecisionState {
                decision: "candidate_applied".to_owned(),
                reason: "candidate is being measured".to_owned(),
                unix_nanos: Some(200),
                score_total: Some(818),
            }),
            degraded: vec![DaemonDegradedStatus {
                category: "data_quality".to_owned(),
                message: "Low: low scored samples".to_owned(),
            }],
            faulted: None,
            ..DaemonState::default()
        };

        let status = status_from_daemon_state(PathBuf::from("/tmp/daemon_state.json"), &state);

        assert_eq!(status.phase, "measure");
        assert_eq!(status.mode, "ApplyLowRisk");
        assert_eq!(
            status.target,
            Some(StatusTarget {
                comm: "KingdomCome.exe".to_owned(),
                pid: 1234,
            })
        );
        assert_eq!(status.active_profile, None);
        assert_eq!(status.active_candidate.as_deref(), Some("game-main"));
        assert_eq!(
            status.last_decision,
            "candidate_applied: candidate is being measured"
        );
        assert!(status.rollback_available);
        assert_eq!(
            status.last_rollback_path.as_deref(),
            Some("/tmp/stutter-restore.json")
        );
        assert!(status.cooldown_remaining_seconds.unwrap_or(0) > 0);
        assert_eq!(status.current_score, Some(818));
        assert_eq!(
            status.data_quality.as_deref(),
            Some("Low: low scored samples")
        );
        assert_eq!(status.manual_restore_command, "stutter restore");
    }

    #[test]
    fn status_from_daemon_state_renders_daemon_lifecycle_labels_and_candidates() {
        let base_state = DaemonState {
            mode: DaemonMode::ApplyLowRisk,
            active_experiment: Some(DaemonExperimentState {
                experiment_id: "experiment-1".to_owned(),
                action_id: "cpu-affinity-profile:game-main".to_owned(),
                candidate_name: Some("game-main".to_owned()),
                safety_class: SafetyClass::ReversibleLowRisk,
                started_unix_nanos: Some(100),
            }),
            ..DaemonState::default()
        };

        let cases = [
            (DaemonPhase::Disabled, "disabled", None, None),
            (DaemonPhase::Init, "init", None, None),
            (DaemonPhase::Recover, "recover", None, None),
            (DaemonPhase::Paused, "paused", None, None),
            (DaemonPhase::Observe, "observe", None, None),
            (DaemonPhase::Decide, "decide", None, Some("game-main")),
            (DaemonPhase::Apply, "apply", None, Some("game-main")),
            (DaemonPhase::Measure, "measure", None, Some("game-main")),
            (DaemonPhase::Keep, "keep", Some("game-main"), None),
            (DaemonPhase::Rollback, "rollback", None, None),
            (DaemonPhase::Cooldown, "cooldown", Some("game-main"), None),
            (DaemonPhase::Faulted, "faulted", None, None),
            (DaemonPhase::Shutdown, "shutdown", None, None),
        ];

        for (phase, expected_phase, expected_profile, expected_candidate) in cases {
            let mut state = base_state.clone();
            state.phase = phase;

            let status = status_from_daemon_state(PathBuf::from("/tmp/daemon_state.json"), &state);

            assert_eq!(status.phase, expected_phase);
            assert_eq!(status.active_profile.as_deref(), expected_profile);
            assert_eq!(status.active_candidate.as_deref(), expected_candidate);
        }
    }

    #[test]
    fn load_autotune_status_prefers_daemon_state_snapshot_when_present() {
        let dir = temp_dir("prefers-daemon-state");
        let history_path = dir.join("history.jsonl");
        let event = kept_event();
        let mut file = fs::File::create(&history_path).unwrap();
        serde_json::to_writer(&mut file, &event).unwrap();
        file.write_all(b"\n").unwrap();

        let daemon_state_path = dir.join("daemon_state.json");
        let state = DaemonState {
            mode: DaemonMode::ApplyLowRisk,
            phase: DaemonPhase::Faulted,
            last_decision: Some(DaemonDecisionState {
                decision: "faulted".to_owned(),
                reason: "snapshot fault wins over history".to_owned(),
                unix_nanos: Some(300),
                score_total: Some(42),
            }),
            faulted: Some(DaemonFaultState {
                reason: "snapshot fault wins over history".to_owned(),
                manual_restore_command: Some("stutter restore".to_owned()),
            }),
            ..DaemonState::default()
        };
        DaemonStateSnapshotWriter::new(&daemon_state_path)
            .write(&state)
            .unwrap();

        let status = load_autotune_status(&history_path).unwrap();

        assert_eq!(status.phase, "faulted");
        assert_eq!(status.mode, "ApplyLowRisk");
        assert_eq!(status.current_score, Some(42));
        assert_eq!(
            status.last_fault.as_deref(),
            Some("snapshot fault wins over history")
        );
        assert_eq!(status.active_profile, None);
        assert_eq!(status.history_path, daemon_state_path);

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn command_reads_history_file_and_renders_text() {
        let dir = temp_dir("command-text");
        let path = dir.join("history.jsonl");
        let event = kept_event();
        let mut file = fs::File::create(&path).unwrap();
        serde_json::to_writer(&mut file, &event).unwrap();
        file.write_all(b"\n").unwrap();

        let status = load_autotune_status(&path).unwrap();

        assert_eq!(status.phase, "Cooldown");
        assert_eq!(
            status.active_profile.as_deref(),
            Some("game-main-suggested")
        );

        fs::remove_dir_all(dir).ok();
    }
}
