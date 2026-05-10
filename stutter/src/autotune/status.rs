use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::history::{
    AutotuneHistoryEvent, AutotuneMode, ControllerPhase, TargetIdentity,
    default_autotune_history_path, read_autotune_history_events,
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

fn cooldown_remaining_seconds_from_events(events: &[AutotuneHistoryEvent]) -> Option<u64> {
    let now = crate::audit::unix_nanos_now();

    for event in events.iter().rev() {
        let Some(cooldown_until) =
            cooldown_until_unix_nanos_from_policy(&event.decision.rollback_policy)
        else {
            continue;
        };

        if now >= cooldown_until {
            return Some(0);
        }

        let remaining_nanos = cooldown_until.saturating_sub(now);
        let remaining_seconds = remaining_nanos.div_ceil(1_000_000_000);
        return Some(remaining_seconds.min(u64::MAX as u128) as u64);
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
        autotune::history::{AutotuneDecisionSummary, ObservationSummary, SituationKind},
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
