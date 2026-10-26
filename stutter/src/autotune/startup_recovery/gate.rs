//! The startup-recovery gate every long-running entry point passes before it plans anything.
use super::{StartupRecoveryConfig, StartupRecoveryOutcome, recover_controller_journal_on_startup};
use crate::autotune::runtime::AutotuneRuntimeConfig;

/// Restore what a previous controller left applied, then report what happened. A fault means a
/// token could not be restored: the caller must not start, because new work would bury it.
pub fn recover_before_start(
    config: StartupRecoveryConfig,
) -> anyhow::Result<StartupRecoveryOutcome> {
    let outcome = recover_controller_journal_on_startup(config)?;
    match &outcome {
        StartupRecoveryOutcome::Recovered {
            experiment_id,
            action_id,
            affected_tasks,
            manual_restore_command,
        } => log::info!(
            "autotune startup recovery: rolled back interrupted action experiment_id={experiment_id} action_id={action_id} affected_tasks={affected_tasks} manual_restore_command=\"{manual_restore_command}\""
        ),
        StartupRecoveryOutcome::Faulted {
            experiment_id,
            action_id,
            manual_restore_command,
            reason,
        } => {
            log::error!(
                "autotune startup recovery faulted: experiment_id={experiment_id} action_id={action_id} reason={reason}"
            );
            log::error!("manual restore command: {manual_restore_command}");
            anyhow::bail!(
                "autotune startup recovery failed; controller is Faulted; manual restore command: {manual_restore_command}"
            );
        }
        StartupRecoveryOutcome::RollbackDisabled {
            experiment_id,
            action_id,
            manual_restore_command,
        } => log::warn!(
            "autotune startup recovery: rollback_on_crash_recovery=false; leaving applied journal in place experiment_id={experiment_id} action_id={action_id} manual_restore_command=\"{manual_restore_command}\""
        ),
        StartupRecoveryOutcome::ApplyingWithoutRollback {
            experiment_id,
            action_id,
        } => log::warn!(
            "autotune startup recovery: found applying journal without rollback token experiment_id={experiment_id} action_id={action_id}; no automatic rollback attempted"
        ),
        StartupRecoveryOutcome::Clean => {}
    }
    Ok(outcome)
}

/// The recovery inputs of a live session: the same journal, history and worker socket it will use.
pub fn startup_recovery_config_for_runtime(
    config: &AutotuneRuntimeConfig,
) -> StartupRecoveryConfig {
    let defaults = StartupRecoveryConfig::default();
    let autotune = &config.daemon_config.autotune;
    StartupRecoveryConfig {
        rollback_on_crash_recovery: autotune.rollback_on_crash_recovery,
        journal_path: config
            .controller_journal_path
            .clone()
            .unwrap_or(defaults.journal_path),
        history_path: config.history_log.clone().unwrap_or(defaults.history_path),
        // A worker that is already listening restores what the controller cannot.
        privileged_worker_socket: autotune
            .privileged_worker_socket
            .clone()
            .or_else(|| crate::daemon::privilege::default_privileged_worker_socket_path().ok())
            .filter(|path| path.exists()),
        ..defaults
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gate_reads_the_session_s_own_journal_and_history() {
        let mut config = AutotuneRuntimeConfig::apply_low_risk(None, Some(1234), None);
        config.controller_journal_path = Some("/run/x/controller.json".into());
        config.history_log = Some("/run/x/history.jsonl".into());
        let recovery = startup_recovery_config_for_runtime(&config);
        assert_eq!(
            recovery.journal_path,
            std::path::PathBuf::from("/run/x/controller.json")
        );
        assert_eq!(
            recovery.history_path,
            std::path::PathBuf::from("/run/x/history.jsonl")
        );
        assert!(recovery.rollback_on_crash_recovery);
    }

    #[test]
    fn a_clean_journal_passes_the_gate_and_a_missing_one_is_clean() {
        let dir = std::env::temp_dir().join(format!(
            "stutter-gate-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let config = StartupRecoveryConfig {
            journal_path: dir.join("controller_journal.json"),
            audit_path: dir.join("audit.jsonl"),
            history_path: dir.join("history.jsonl"),
            state_snapshot_path: dir.join("state.json"),
            ..StartupRecoveryConfig::default()
        };
        assert_eq!(
            recover_before_start(config).unwrap(),
            StartupRecoveryOutcome::Clean
        );
    }
}
