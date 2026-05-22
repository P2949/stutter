use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::{
    commands::input::DaemonResyncStateCommandInput,
    daemon::state::{
        DaemonDecisionState, DaemonPhase, DaemonState, DaemonStateSnapshotWriter,
        default_daemon_state_snapshot_path, load_daemon_state,
    },
};

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct DaemonResyncStateOutcome {
    pub dry_run: bool,
    pub state_path: PathBuf,
    pub state_loaded: bool,
    pub abandoned_active_experiment: bool,
    pub abandoned_active_rollback: bool,
    pub abandoned_profile_count: usize,
    pub wrote_state: bool,
    pub decision: String,
}

pub fn run_resync_state_command(input: DaemonResyncStateCommandInput) -> anyhow::Result<()> {
    let state_path = default_daemon_state_snapshot_path();
    let outcome = run_resync_state_command_with_path(&state_path, input.dry_run)?;

    if input.json {
        println!("{}", serde_json::to_string_pretty(&outcome)?);
    } else {
        println!("{}", render_resync_state_text(&outcome));
    }

    Ok(())
}

pub fn run_resync_state_command_with_path(
    state_path: &Path,
    dry_run: bool,
) -> anyhow::Result<DaemonResyncStateOutcome> {
    let state_loaded = state_path.exists();
    let state = if state_loaded {
        load_daemon_state(state_path)?
    } else {
        DaemonState::default()
    };

    let abandoned_active_experiment = state.active_experiment.is_some();
    let abandoned_active_rollback = state.active_rollback.is_some();
    let abandoned_profile_count = state.profile_memory.profiles.len();
    let mut resynced = state;
    resynced.active_experiment = None;
    resynced.active_rollback = None;
    resynced.cooldown_until_unix_nanos = None;
    resynced.faulted = None;
    resynced.degraded.clear();
    resynced.profile_memory.profiles.clear();
    resynced.phase = DaemonPhase::Observe;
    resynced.last_decision = Some(DaemonDecisionState {
        decision: "daemon_resync_state".to_owned(),
        reason: "accepted external mutation and abandoned stale daemon action state".to_owned(),
        unix_nanos: Some(crate::audit::unix_nanos_now()),
        diagnostic_score_total: None,
        candidate_count: None,
        top_denied_reason: None,
        planner: None,
        situation: None,
        focus_kind: None,
    });

    let wrote_state = !dry_run;
    if !dry_run {
        DaemonStateSnapshotWriter::new(state_path).write(&resynced)?;
    }

    Ok(DaemonResyncStateOutcome {
        dry_run,
        state_path: state_path.to_path_buf(),
        state_loaded,
        abandoned_active_experiment,
        abandoned_active_rollback,
        abandoned_profile_count,
        wrote_state,
        decision: resynced
            .last_decision
            .as_ref()
            .map(|decision| decision.decision.clone())
            .unwrap_or_else(|| "daemon_resync_state".to_owned()),
    })
}

pub fn render_resync_state_text(outcome: &DaemonResyncStateOutcome) -> String {
    format!(
        "daemon resync-state summary: dry_run={} state_loaded={} wrote_state={} abandoned_active_experiment={} abandoned_active_rollback={} abandoned_profiles={} state_path={}\n",
        outcome.dry_run,
        outcome.state_loaded,
        outcome.wrote_state,
        outcome.abandoned_active_experiment,
        outcome.abandoned_active_rollback,
        outcome.abandoned_profile_count,
        outcome.state_path.display()
    )
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::{
        actions::SafetyClass,
        daemon::{
            policy::DaemonMode,
            state::{DaemonExperimentState, DaemonProfileMemory, DaemonWorkloadProfile},
        },
    };

    fn temp_state_path(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "stutter-resync-state-{name}-{}-{}.json",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        path
    }

    #[test]
    fn resync_state_dry_run_reports_without_writing() {
        let path = temp_state_path("dry-run");
        let outcome = run_resync_state_command_with_path(&path, true).unwrap();

        assert!(outcome.dry_run);
        assert!(!outcome.state_loaded);
        assert!(!outcome.wrote_state);
        assert!(!path.exists());
    }

    #[test]
    fn resync_state_abandons_stale_action_state() {
        let path = temp_state_path("apply");
        let mut state = DaemonState {
            mode: DaemonMode::ApplyLowRisk,
            phase: DaemonPhase::Faulted,
            active_experiment: Some(DaemonExperimentState {
                experiment_id: "exp".to_owned(),
                action_id: "action".to_owned(),
                candidate_name: Some("candidate".to_owned()),
                mode: DaemonMode::ApplyLowRisk,
                safety_class: SafetyClass::ReversibleLowRisk,
                started_unix_nanos: Some(1),
            }),
            profile_memory: DaemonProfileMemory {
                profiles: vec![DaemonWorkloadProfile {
                    workload_identity_hash: "hash".to_owned(),
                    workload_label: None,
                    candidate_name: "candidate".to_owned(),
                    action_id: "action".to_owned(),
                    action_kind: "fake".to_owned(),
                    safety_class: SafetyClass::ReversibleLowRisk,
                    kept_unix_nanos: 1,
                    last_validated_unix_nanos: Some(1),
                    diagnostic_baseline_diagnostic_score_total: None,
                    diagnostic_candidate_diagnostic_score_total: None,
                    score_delta: 0,
                    confidence_milli: 1000,
                    environment: Default::default(),
                    partition: Default::default(),
                }],
            },
            ..DaemonState::default()
        };
        state.active_rollback = Some(crate::daemon::state::DaemonRollbackState {
            action_id: "action".to_owned(),
            mode: DaemonMode::ApplyLowRisk,
            safety_class: SafetyClass::ReversibleLowRisk,
            rollback_available: true,
            token: None,
            manual_restore_command: Some("restore".to_owned()),
        });
        DaemonStateSnapshotWriter::new(&path).write(&state).unwrap();

        let outcome = run_resync_state_command_with_path(&path, false).unwrap();
        let loaded = load_daemon_state(&path).unwrap();

        assert!(outcome.wrote_state);
        assert!(outcome.abandoned_active_experiment);
        assert!(outcome.abandoned_active_rollback);
        assert_eq!(outcome.abandoned_profile_count, 1);
        assert_eq!(loaded.phase, DaemonPhase::Observe);
        assert!(loaded.active_experiment.is_none());
        assert!(loaded.active_rollback.is_none());
        assert!(loaded.profile_memory.profiles.is_empty());

        fs::remove_file(path).ok();
    }
}
