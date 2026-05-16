use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::{
    actions::RollbackToken,
    audit::{AuditEvent, append_audit_event_to_path},
    autotune::{
        controller_journal::{
            ControllerJournalRecord, ControllerJournalState, default_controller_journal_path,
            read_controller_journal, write_controller_journal_clean,
        },
        history::{
            AutotuneDecisionSummary, AutotuneHistoryEvent, AutotuneMode, ControllerPhase,
            ObservationSummary, SituationKind, append_autotune_history_event,
            default_autotune_history_path,
        },
    },
    daemon::{
        DaemonPhase, DaemonState, DaemonStateSnapshotWriter, DaemonStateStore,
        StartupRecoveryDaemonStateInput, daemon_state_for_startup_recovery_snapshot,
        default_daemon_state_snapshot_path, safety_class_for_rollback_token,
    },
};

#[derive(Clone, Debug)]
pub struct StartupRecoveryConfig {
    pub rollback_on_crash_recovery: bool,
    pub journal_path: PathBuf,
    pub audit_path: PathBuf,
    pub history_path: PathBuf,
    pub state_snapshot_path: PathBuf,
}

impl Default for StartupRecoveryConfig {
    fn default() -> Self {
        Self {
            rollback_on_crash_recovery: true,
            journal_path: default_controller_journal_path(),
            audit_path: crate::audit::default_audit_log_path(),
            history_path: default_autotune_history_path(),
            state_snapshot_path: default_daemon_state_snapshot_path(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartupRecoveryOutcome {
    Clean,
    ApplyingWithoutRollback {
        experiment_id: String,
        action_id: String,
    },
    RollbackDisabled {
        experiment_id: String,
        action_id: String,
        manual_restore_command: String,
    },
    Recovered {
        experiment_id: String,
        action_id: String,
        affected_tasks: usize,
        manual_restore_command: String,
    },
    Faulted {
        experiment_id: String,
        action_id: String,
        manual_restore_command: String,
        reason: String,
    },
}

impl StartupRecoveryOutcome {
    pub fn is_faulted(&self) -> bool {
        matches!(self, Self::Faulted { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartupRecoveryRollbackSummary {
    pub affected_tasks: usize,
    pub message: String,
}

pub trait StartupRecoveryRollbackExecutor {
    fn rollback(&mut self, token: &RollbackToken)
    -> anyhow::Result<StartupRecoveryRollbackSummary>;
}

#[derive(Default)]
pub struct RealStartupRecoveryRollbackExecutor;

impl StartupRecoveryRollbackExecutor for RealStartupRecoveryRollbackExecutor {
    fn rollback(
        &mut self,
        token: &RollbackToken,
    ) -> anyhow::Result<StartupRecoveryRollbackSummary> {
        let summary = crate::autotune::emergency_restore::restore_rollback_token(token)
            .with_context(|| {
                format!(
                    "startup crash recovery failed to restore rollback token_kind={}",
                    rollback_token_kind(token)
                )
            })?;

        let mut message = format!(
            "rollback_kind={} restored_items={} skipped_items={}",
            summary.rollback_kind, summary.restored_items, summary.skipped_items
        );
        if !summary.messages.is_empty() {
            message.push_str(" messages=\"");
            message.push_str(&summary.messages.join("; "));
            message.push('"');
        }

        Ok(StartupRecoveryRollbackSummary {
            affected_tasks: summary.restored_items,
            message,
        })
    }
}

pub fn recover_controller_journal_on_startup(
    config: StartupRecoveryConfig,
) -> anyhow::Result<StartupRecoveryOutcome> {
    let mut executor = RealStartupRecoveryRollbackExecutor;
    recover_controller_journal_with_executor(config, &mut executor)
}

pub fn recover_controller_journal_with_executor<E: StartupRecoveryRollbackExecutor + ?Sized>(
    config: StartupRecoveryConfig,
    executor: &mut E,
) -> anyhow::Result<StartupRecoveryOutcome> {
    let record = read_controller_journal(&config.journal_path)?;

    match record.state() {
        ControllerJournalState::Clean => Ok(StartupRecoveryOutcome::Clean),
        ControllerJournalState::Reverted => {
            write_controller_journal_clean(&config.journal_path).with_context(
                || "failed to clear reverted controller journal phase during startup recovery",
            )?;
            Ok(StartupRecoveryOutcome::Clean)
        }
        ControllerJournalState::Planned | ControllerJournalState::Preflighted => {
            write_controller_journal_clean(&config.journal_path).with_context(|| {
                format!(
                    "failed to clear pre-apply controller journal phase={} during startup recovery",
                    record.state().as_str()
                )
            })?;
            Ok(StartupRecoveryOutcome::Clean)
        }
        ControllerJournalState::Applying => {
            let (experiment_id, action_id) = journal_experiment_action(&record);
            let reason =
                "startup crash recovery found applying journal without rollback token".to_owned();
            write_startup_recovery_daemon_state_snapshot(
                &config.state_snapshot_path,
                startup_recovery_daemon_state(
                    DaemonPhase::Faulted,
                    "applying_without_rollback",
                    reason.clone(),
                    &experiment_id,
                    &action_id,
                    None,
                    false,
                    true,
                    true,
                    Some("stutter daemon emergency-restore"),
                ),
            )?;

            Ok(StartupRecoveryOutcome::ApplyingWithoutRollback {
                experiment_id,
                action_id,
            })
        }
        ControllerJournalState::Applied
        | ControllerJournalState::Verifying
        | ControllerJournalState::Measuring
        | ControllerJournalState::Keeping
        | ControllerJournalState::Reverting
        | ControllerJournalState::Faulted => {
            let (experiment_id, action_id) = journal_experiment_action(&record);
            if let Some(rollback_token) = record.rollback_token().cloned() {
                recover_applied_journal_record(
                    config,
                    executor,
                    experiment_id,
                    action_id,
                    rollback_token,
                )
            } else {
                let reason = format!(
                    "startup crash recovery found {} journal without rollback token",
                    record.state().as_str()
                );
                write_startup_recovery_daemon_state_snapshot(
                    &config.state_snapshot_path,
                    startup_recovery_daemon_state(
                        DaemonPhase::Faulted,
                        "missing_rollback_token",
                        reason,
                        &experiment_id,
                        &action_id,
                        None,
                        false,
                        true,
                        true,
                        Some("stutter daemon emergency-restore"),
                    ),
                )?;

                Ok(StartupRecoveryOutcome::ApplyingWithoutRollback {
                    experiment_id,
                    action_id,
                })
            }
        }
    }
}

fn journal_experiment_action(record: &ControllerJournalRecord) -> (String, String) {
    let state = record.state().as_str();
    (
        record
            .experiment_id
            .clone()
            .unwrap_or_else(|| format!("{state}-unknown-experiment")),
        record
            .action_id
            .clone()
            .unwrap_or_else(|| format!("{state}-unknown-action")),
    )
}

fn recover_applied_journal_record<E: StartupRecoveryRollbackExecutor + ?Sized>(
    config: StartupRecoveryConfig,
    executor: &mut E,
    experiment_id: String,
    action_id: String,
    rollback_token: RollbackToken,
) -> anyhow::Result<StartupRecoveryOutcome> {
    let manual_restore_command = manual_restore_command_for_token(&rollback_token);

    if !config.rollback_on_crash_recovery {
        let reason = format!(
            "startup crash recovery rollback disabled; manual_restore_command=\"{}\"",
            manual_restore_command
        );
        write_startup_recovery_daemon_state_snapshot(
            &config.state_snapshot_path,
            startup_recovery_daemon_state(
                DaemonPhase::Faulted,
                "rollback_disabled",
                reason,
                &experiment_id,
                &action_id,
                Some(&rollback_token),
                true,
                true,
                true,
                Some(&manual_restore_command),
            ),
        )?;

        return Ok(StartupRecoveryOutcome::RollbackDisabled {
            experiment_id,
            action_id,
            manual_restore_command,
        });
    }

    match executor.rollback(&rollback_token) {
        Ok(summary) => {
            write_controller_journal_clean(&config.journal_path).with_context(|| {
                format!(
                    "failed to clear controller journal after crash-recovery rollback for action_id={}",
                    action_id
                )
            })?;

            write_startup_recovery_audit_event(
                &config.audit_path,
                &action_id,
                &rollback_token,
                true,
                summary.affected_tasks,
                format!(
                    "startup crash recovery rollback succeeded experiment_id={} action_id={} {}",
                    experiment_id, action_id, summary.message
                ),
            )?;

            write_startup_recovery_history_event(
                &config.history_path,
                ControllerPhase::Cooldown,
                "restored",
                &experiment_id,
                &action_id,
                &rollback_token,
                true,
                format!(
                    "startup crash recovery rollback succeeded; manual_restore_command=\"{}\"",
                    manual_restore_command
                ),
            )?;

            write_startup_recovery_daemon_state_snapshot(
                &config.state_snapshot_path,
                startup_recovery_daemon_state(
                    DaemonPhase::Cooldown,
                    "restored",
                    format!(
                        "startup crash recovery rollback succeeded; affected_tasks={}",
                        summary.affected_tasks
                    ),
                    &experiment_id,
                    &action_id,
                    None,
                    false,
                    false,
                    false,
                    Some(&manual_restore_command),
                ),
            )?;

            Ok(StartupRecoveryOutcome::Recovered {
                experiment_id,
                action_id,
                affected_tasks: summary.affected_tasks,
                manual_restore_command,
            })
        }
        Err(err) => {
            let reason = format!("startup crash recovery rollback failed: {err:#}");

            write_startup_recovery_audit_event(
                &config.audit_path,
                &action_id,
                &rollback_token,
                false,
                0,
                format!(
                    "{}; manual_restore_command=\"{}\"",
                    reason, manual_restore_command
                ),
            )?;

            write_startup_recovery_history_event(
                &config.history_path,
                ControllerPhase::Faulted,
                "CrashRecoveryFault",
                &experiment_id,
                &action_id,
                &rollback_token,
                false,
                format!(
                    "{}; manual_restore_command=\"{}\"",
                    reason, manual_restore_command
                ),
            )?;

            write_startup_recovery_daemon_state_snapshot(
                &config.state_snapshot_path,
                startup_recovery_daemon_state(
                    DaemonPhase::Faulted,
                    "faulted",
                    reason.clone(),
                    &experiment_id,
                    &action_id,
                    Some(&rollback_token),
                    true,
                    true,
                    true,
                    Some(&manual_restore_command),
                ),
            )?;

            Ok(StartupRecoveryOutcome::Faulted {
                experiment_id,
                action_id,
                manual_restore_command,
                reason,
            })
        }
    }
}

fn write_startup_recovery_audit_event(
    audit_path: &Path,
    action_id: &str,
    rollback_token: &RollbackToken,
    success: bool,
    affected_tasks: usize,
    message: String,
) -> anyhow::Result<()> {
    let event = AuditEvent {
        schema_version: 1,
        unix_nanos: crate::audit::unix_nanos_now(),
        command: "autotune-startup-recovery".to_owned(),
        action_id: Some(action_id.to_owned()),
        safety_class: Some(safety_class_for_rollback_token(rollback_token)),
        dry_run: false,
        success,
        affected_tasks,
        restore_path: rollback_token.restore_path().cloned(),
        action_phase: None,
        error_category: None,
        message,
    };

    append_audit_event_to_path(audit_path, &event).with_context(|| {
        format!(
            "failed to write startup recovery audit event to {}",
            audit_path.display()
        )
    })
}

fn write_startup_recovery_history_event(
    history_path: &Path,
    phase: ControllerPhase,
    decision: &str,
    experiment_id: &str,
    action_id: &str,
    rollback_token: &RollbackToken,
    rollback_performed: bool,
    reason: String,
) -> anyhow::Result<()> {
    let event = AutotuneHistoryEvent::new(
        "startup-recovery",
        phase,
        AutotuneMode::ApplyLowRisk,
        None,
        SituationKind::Unknown,
        empty_observation_summary(),
        AutotuneDecisionSummary {
            decision: decision.to_owned(),
            candidate_name: candidate_name_from_action_id(action_id),
            action_kind: Some(action_kind_from_action_id(action_id)),
            safety_class: Some(safety_class_for_rollback_token(rollback_token)),
            eligible: rollback_performed,
            rollback_policy: "rollback-on-crash-recovery".to_owned(),
        },
        reason,
    )
    .with_experiment_id(experiment_id.to_owned())
    .with_action_id(action_id.to_owned())
    .with_rollback_performed(rollback_performed);

    append_autotune_history_event(history_path, &event).with_context(|| {
        format!(
            "failed to write startup recovery history event to {}",
            history_path.display()
        )
    })
}

fn write_startup_recovery_daemon_state_snapshot(
    state_snapshot_path: &Path,
    state: DaemonState,
) -> anyhow::Result<()> {
    let writer = DaemonStateSnapshotWriter::new(state_snapshot_path);
    let mut store = DaemonStateStore::new(DaemonState::default(), Some(writer));

    store.replace(state).with_context(|| {
        format!(
            "failed to write startup recovery daemon state snapshot to {}",
            state_snapshot_path.display()
        )
    })
}

#[allow(clippy::too_many_arguments)]
fn startup_recovery_daemon_state(
    phase: DaemonPhase,
    decision: &str,
    reason: String,
    experiment_id: &str,
    action_id: &str,
    rollback_token: Option<&RollbackToken>,
    rollback_available: bool,
    include_active_experiment: bool,
    faulted: bool,
    manual_restore_command: Option<&str>,
) -> DaemonState {
    daemon_state_for_startup_recovery_snapshot(StartupRecoveryDaemonStateInput {
        phase,
        decision,
        reason,
        experiment_id,
        action_id,
        rollback_token,
        rollback_available,
        include_active_experiment,
        faulted,
        manual_restore_command,
    })
}

fn empty_observation_summary() -> ObservationSummary {
    ObservationSummary {
        target_present: false,
        active_target_count: 0,
        scored_task_count: 0,
        interval_count: 0,
        scored_samples: 0,
        score_total: 0,
        over_1ms: 0,
        over_2ms: 0,
        over_5ms: 0,
        frame_p99_ms: 0.0,
        frame_max_ms: 0.0,
        drop_counter_total: 0,
        data_quality: "Unknown".to_owned(),
    }
}

pub fn manual_restore_command_for_token(token: &RollbackToken) -> String {
    crate::autotune::emergency_restore::manual_restore_command_for_token(token)
}

fn rollback_token_kind(token: &RollbackToken) -> &'static str {
    match token {
        RollbackToken::CpuAffinityRestoreFile { .. } => "cpu-affinity-restore-file",
        RollbackToken::NiceRestore { .. } => "nice-restore",
        RollbackToken::IrqAffinityRestore { .. } => "irq-affinity-restore",
        RollbackToken::IoPrioRestore { .. } => "ioprio-restore",
        RollbackToken::UclampRestore { .. } => "uclamp-restore",
        RollbackToken::CgroupRestore { .. } => "cgroup-restore",
        RollbackToken::CpuPowerRestore { .. } => "cpu-power-restore",
        RollbackToken::VmKnobRestore { .. } => "vm-knob-restore",
        RollbackToken::GpuPowerRestore { .. } => "gpu-power-restore",
        RollbackToken::SysfsRestore { .. } => "sysfs-restore",
    }
}

fn action_kind_from_action_id(action_id: &str) -> String {
    let kind = action_id
        .split_once(':')
        .map(|(kind, _)| kind)
        .unwrap_or(action_id);

    kind.replace('-', "_")
}

fn candidate_name_from_action_id(action_id: &str) -> Option<String> {
    action_id
        .split_once(':')
        .map(|(_, candidate)| candidate.to_owned())
        .filter(|candidate| !candidate.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::{
        actions::{
            CgroupRestoreRecord, CpuPowerRestoreRecord, GpuPowerRestoreRecord, IoPrioRestoreRecord,
            IrqAffinityRestoreRecord, NiceRestoreRecord, UclampRestoreRecord, VmKnobRestoreRecord,
        },
        autotune::controller_journal::{
            ControllerJournalRecord, ControllerJournalState, write_controller_journal_applied,
            write_controller_journal_applying, write_controller_journal_clean,
            write_controller_journal_record,
        },
    };

    #[derive(Default)]
    struct FakeRollbackExecutor {
        calls: usize,
        fail: bool,
        affected_tasks: usize,
    }

    impl StartupRecoveryRollbackExecutor for FakeRollbackExecutor {
        fn rollback(
            &mut self,
            _token: &RollbackToken,
        ) -> anyhow::Result<StartupRecoveryRollbackSummary> {
            self.calls += 1;

            if self.fail {
                anyhow::bail!("intentional recovery rollback failure");
            }

            Ok(StartupRecoveryRollbackSummary {
                affected_tasks: self.affected_tasks,
                message: format!("fake restored={}", self.affected_tasks),
            })
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-startup-recovery-test-{name}-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn rollback_token() -> RollbackToken {
        RollbackToken::CpuAffinityRestoreFile {
            path: crate::affinity::default_restore_path(),
            affected_tasks: 31,
        }
    }

    fn config_for_dir(dir: &Path, rollback_on_crash_recovery: bool) -> StartupRecoveryConfig {
        StartupRecoveryConfig {
            rollback_on_crash_recovery,
            journal_path: dir.join("controller_journal.json"),
            audit_path: dir.join("audit.jsonl"),
            history_path: dir.join("history.jsonl"),
            state_snapshot_path: dir.join("daemon_state.json"),
        }
    }

    fn read_daemon_state_snapshot(path: &Path) -> DaemonState {
        crate::daemon::load_daemon_state(path).unwrap()
    }

    #[test]
    fn clean_journal_needs_no_recovery() {
        let dir = temp_dir("clean");
        let config = config_for_dir(&dir, true);
        write_controller_journal_clean(&config.journal_path).unwrap();
        let mut executor = FakeRollbackExecutor::default();

        let outcome = recover_controller_journal_with_executor(config, &mut executor).unwrap();

        assert_eq!(outcome, StartupRecoveryOutcome::Clean);
        assert_eq!(executor.calls, 0);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn pre_apply_transaction_phase_cleans_without_rollback() {
        let dir = temp_dir("planned");
        let config = config_for_dir(&dir, true);
        let record = ControllerJournalRecord::for_phase(
            ControllerJournalState::Planned,
            "experiment-1",
            "cpu-affinity-profile:game-main",
            None,
        )
        .with_candidate("game-main");
        write_controller_journal_record(&config.journal_path, &record).unwrap();
        let mut executor = FakeRollbackExecutor::default();

        let outcome =
            recover_controller_journal_with_executor(config.clone(), &mut executor).unwrap();

        assert_eq!(outcome, StartupRecoveryOutcome::Clean);
        assert_eq!(executor.calls, 0);
        assert!(
            crate::autotune::controller_journal::read_controller_journal(&config.journal_path)
                .unwrap()
                .is_clean()
        );
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn post_apply_transaction_phase_rolls_back_on_startup() {
        let dir = temp_dir("verifying");
        let config = config_for_dir(&dir, true);
        let record = ControllerJournalRecord::for_phase(
            ControllerJournalState::Verifying,
            "experiment-1",
            "cpu-affinity-profile:game-main",
            Some(rollback_token()),
        )
        .with_verify_result("pending");
        write_controller_journal_record(&config.journal_path, &record).unwrap();
        let mut executor = FakeRollbackExecutor {
            calls: 0,
            fail: false,
            affected_tasks: 31,
        };

        let outcome =
            recover_controller_journal_with_executor(config.clone(), &mut executor).unwrap();

        assert_eq!(
            outcome,
            StartupRecoveryOutcome::Recovered {
                experiment_id: "experiment-1".to_owned(),
                action_id: "cpu-affinity-profile:game-main".to_owned(),
                affected_tasks: 31,
                manual_restore_command: "stutter restore".to_owned(),
            }
        );
        assert_eq!(executor.calls, 1);
        assert!(
            crate::autotune::controller_journal::read_controller_journal(&config.journal_path)
                .unwrap()
                .is_clean()
        );
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn live_runtime_transaction_phases_roll_back_on_startup() {
        for phase in [
            ControllerJournalState::Measuring,
            ControllerJournalState::Keeping,
            ControllerJournalState::Reverting,
        ] {
            let dir = temp_dir(phase.as_str());
            let config = config_for_dir(&dir, true);
            let record = ControllerJournalRecord::for_phase(
                phase,
                "experiment-live",
                "cpu-affinity-profile:game-main",
                Some(rollback_token()),
            )
            .with_candidate("game-main")
            .with_verify_result("phase_written_by_runtime");
            write_controller_journal_record(&config.journal_path, &record).unwrap();
            let mut executor = FakeRollbackExecutor {
                calls: 0,
                fail: false,
                affected_tasks: 31,
            };

            let outcome =
                recover_controller_journal_with_executor(config.clone(), &mut executor).unwrap();

            assert_eq!(
                outcome,
                StartupRecoveryOutcome::Recovered {
                    experiment_id: "experiment-live".to_owned(),
                    action_id: "cpu-affinity-profile:game-main".to_owned(),
                    affected_tasks: 31,
                    manual_restore_command: "stutter restore".to_owned(),
                }
            );
            assert_eq!(executor.calls, 1);
            assert!(
                crate::autotune::controller_journal::read_controller_journal(&config.journal_path)
                    .unwrap()
                    .is_clean()
            );
            fs::remove_dir_all(dir).ok();
        }
    }

    #[test]
    fn reverted_transaction_phase_cleans_without_rollback() {
        let dir = temp_dir("reverted");
        let config = config_for_dir(&dir, true);
        let record = ControllerJournalRecord::for_phase(
            ControllerJournalState::Reverted,
            "experiment-live",
            "cpu-affinity-profile:game-main",
            Some(rollback_token()),
        );
        write_controller_journal_record(&config.journal_path, &record).unwrap();
        let mut executor = FakeRollbackExecutor {
            calls: 0,
            fail: false,
            affected_tasks: 31,
        };

        let outcome =
            recover_controller_journal_with_executor(config.clone(), &mut executor).unwrap();

        assert_eq!(outcome, StartupRecoveryOutcome::Clean);
        assert_eq!(executor.calls, 0);
        assert!(
            crate::autotune::controller_journal::read_controller_journal(&config.journal_path)
                .unwrap()
                .is_clean()
        );
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn applying_journal_without_rollback_token_does_not_attempt_recovery() {
        let dir = temp_dir("applying");
        let config = config_for_dir(&dir, true);
        write_controller_journal_applying(
            &config.journal_path,
            "experiment-1",
            "cpu-affinity-profile:game-main",
        )
        .unwrap();
        let mut executor = FakeRollbackExecutor::default();
        let outcome =
            recover_controller_journal_with_executor(config.clone(), &mut executor).unwrap();

        assert_eq!(
            outcome,
            StartupRecoveryOutcome::ApplyingWithoutRollback {
                experiment_id: "experiment-1".to_owned(),
                action_id: "cpu-affinity-profile:game-main".to_owned(),
            }
        );
        assert_eq!(executor.calls, 0);

        let state = read_daemon_state_snapshot(&config.state_snapshot_path);
        assert_eq!(state.phase, DaemonPhase::Faulted);
        assert_eq!(
            state
                .active_experiment
                .as_ref()
                .map(|experiment| experiment.action_id.as_str()),
            Some("cpu-affinity-profile:game-main")
        );
        assert!(state.active_rollback.is_none());
        assert_eq!(
            state
                .faulted
                .as_ref()
                .and_then(|fault| fault.manual_restore_command.as_deref()),
            Some("stutter daemon emergency-restore")
        );
        assert!(
            state
                .faulted
                .as_ref()
                .map(|fault| fault.reason.contains("without rollback token"))
                .unwrap_or(false)
        );

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn applied_journal_rolls_back_audits_history_and_cleans_journal() {
        let dir = temp_dir("applied-success");
        let config = config_for_dir(&dir, true);
        write_controller_journal_applied(
            &config.journal_path,
            "experiment-1",
            "cpu-affinity-profile:game-main",
            rollback_token(),
        )
        .unwrap();
        let mut executor = FakeRollbackExecutor {
            calls: 0,
            fail: false,
            affected_tasks: 31,
        };

        let outcome =
            recover_controller_journal_with_executor(config.clone(), &mut executor).unwrap();

        assert_eq!(
            outcome,
            StartupRecoveryOutcome::Recovered {
                experiment_id: "experiment-1".to_owned(),
                action_id: "cpu-affinity-profile:game-main".to_owned(),
                affected_tasks: 31,
                manual_restore_command: "stutter restore".to_owned(),
            }
        );
        assert_eq!(executor.calls, 1);
        assert!(
            crate::autotune::controller_journal::read_controller_journal(&config.journal_path)
                .unwrap()
                .is_clean()
        );

        let audit = crate::audit::read_audit_tail(&config.audit_path, 10).unwrap();
        assert_eq!(audit.len(), 1);
        assert!(audit[0].success);
        assert_eq!(audit[0].command, "autotune-startup-recovery");
        assert_eq!(
            audit[0].action_id.as_deref(),
            Some("cpu-affinity-profile:game-main")
        );
        assert_eq!(audit[0].affected_tasks, 31);

        let history =
            crate::autotune::history::read_autotune_history_events(&config.history_path).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].phase, ControllerPhase::Cooldown);
        assert_eq!(history[0].decision.decision, "restored");
        assert!(history[0].rollback_performed);

        let state = read_daemon_state_snapshot(&config.state_snapshot_path);
        assert_eq!(state.phase, DaemonPhase::Cooldown);
        assert_eq!(
            state
                .last_decision
                .as_ref()
                .map(|decision| decision.decision.as_str()),
            Some("restored")
        );
        assert!(state.active_experiment.is_none());
        assert!(state.active_rollback.is_none());
        assert!(state.faulted.is_none());

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn applied_journal_with_recovery_disabled_leaves_journal_applied() {
        let dir = temp_dir("disabled");
        let config = config_for_dir(&dir, false);
        write_controller_journal_applied(
            &config.journal_path,
            "experiment-1",
            "cpu-affinity-profile:game-main",
            rollback_token(),
        )
        .unwrap();
        let mut executor = FakeRollbackExecutor::default();

        let outcome =
            recover_controller_journal_with_executor(config.clone(), &mut executor).unwrap();

        assert_eq!(
            outcome,
            StartupRecoveryOutcome::RollbackDisabled {
                experiment_id: "experiment-1".to_owned(),
                action_id: "cpu-affinity-profile:game-main".to_owned(),
                manual_restore_command: "stutter restore".to_owned(),
            }
        );
        assert_eq!(executor.calls, 0);
        assert!(
            !crate::autotune::controller_journal::read_controller_journal(&config.journal_path)
                .unwrap()
                .is_clean()
        );

        let state = read_daemon_state_snapshot(&config.state_snapshot_path);
        assert_eq!(state.phase, DaemonPhase::Faulted);
        assert_eq!(
            state
                .last_decision
                .as_ref()
                .map(|decision| decision.decision.as_str()),
            Some("rollback_disabled")
        );
        assert_eq!(
            state
                .active_rollback
                .as_ref()
                .map(|rollback| rollback.rollback_available),
            Some(true)
        );
        assert!(
            state
                .faulted
                .as_ref()
                .map(|fault| fault.reason.contains("rollback disabled"))
                .unwrap_or(false)
        );

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn rollback_failure_enters_faulted_and_keeps_applied_journal() {
        let dir = temp_dir("rollback-failure");
        let config = config_for_dir(&dir, true);
        write_controller_journal_applied(
            &config.journal_path,
            "experiment-1",
            "cpu-affinity-profile:game-main",
            rollback_token(),
        )
        .unwrap();
        let mut executor = FakeRollbackExecutor {
            calls: 0,
            fail: true,
            affected_tasks: 0,
        };

        let outcome =
            recover_controller_journal_with_executor(config.clone(), &mut executor).unwrap();

        match outcome {
            StartupRecoveryOutcome::Faulted {
                experiment_id,
                action_id,
                manual_restore_command,
                reason,
            } => {
                assert_eq!(experiment_id, "experiment-1");
                assert_eq!(action_id, "cpu-affinity-profile:game-main");
                assert_eq!(manual_restore_command, "stutter restore");
                assert!(reason.contains("intentional recovery rollback failure"));
            }
            other => panic!("expected Faulted recovery outcome, got {other:?}"),
        }

        assert_eq!(executor.calls, 1);
        assert!(
            !crate::autotune::controller_journal::read_controller_journal(&config.journal_path)
                .unwrap()
                .is_clean()
        );

        let audit = crate::audit::read_audit_tail(&config.audit_path, 10).unwrap();
        assert_eq!(audit.len(), 1);
        assert!(!audit[0].success);
        assert!(
            audit[0]
                .message
                .contains("manual_restore_command=\"stutter restore\"")
        );

        let history =
            crate::autotune::history::read_autotune_history_events(&config.history_path).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].phase, ControllerPhase::Faulted);
        assert_eq!(history[0].decision.decision, "CrashRecoveryFault");
        assert!(!history[0].rollback_performed);

        let state = read_daemon_state_snapshot(&config.state_snapshot_path);
        assert_eq!(state.phase, DaemonPhase::Faulted);
        assert_eq!(
            state
                .last_decision
                .as_ref()
                .map(|decision| decision.decision.as_str()),
            Some("faulted")
        );
        assert_eq!(
            state
                .active_rollback
                .as_ref()
                .map(|rollback| rollback.rollback_available),
            Some(true)
        );
        assert!(
            state
                .faulted
                .as_ref()
                .map(|fault| fault
                    .reason
                    .contains("intentional recovery rollback failure"))
                .unwrap_or(false)
        );

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn journal_applied_state_rolls_back_on_start() {
        let dir = temp_dir("phase-15-5-applied-rolls-back");
        let config = config_for_dir(&dir, true);

        write_controller_journal_applied(
            &config.journal_path,
            "experiment-1",
            "cpu-affinity-profile:game-main",
            rollback_token(),
        )
        .unwrap();

        let mut executor = FakeRollbackExecutor {
            calls: 0,
            fail: false,
            affected_tasks: 31,
        };

        let outcome =
            recover_controller_journal_with_executor(config.clone(), &mut executor).unwrap();

        assert_eq!(
            outcome,
            StartupRecoveryOutcome::Recovered {
                experiment_id: "experiment-1".to_owned(),
                action_id: "cpu-affinity-profile:game-main".to_owned(),
                affected_tasks: 31,
                manual_restore_command: "stutter restore".to_owned(),
            }
        );
        assert_eq!(executor.calls, 1);

        assert!(
            crate::autotune::controller_journal::read_controller_journal(&config.journal_path)
                .unwrap()
                .is_clean()
        );

        let audit = crate::audit::read_audit_tail(&config.audit_path, 10).unwrap();
        assert_eq!(audit.len(), 1);
        assert!(audit[0].success);
        assert_eq!(audit[0].command, "autotune-startup-recovery");
        assert_eq!(
            audit[0].action_id.as_deref(),
            Some("cpu-affinity-profile:game-main")
        );
        assert_eq!(audit[0].affected_tasks, 31);
        assert!(
            audit[0]
                .message
                .contains("startup crash recovery rollback succeeded")
        );

        let history =
            crate::autotune::history::read_autotune_history_events(&config.history_path).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].phase, ControllerPhase::Cooldown);
        assert_eq!(history[0].decision.decision, "restored");
        assert_eq!(
            history[0].action_id.as_deref(),
            Some("cpu-affinity-profile:game-main")
        );
        assert_eq!(history[0].experiment_id.as_deref(), Some("experiment-1"));
        assert!(history[0].rollback_performed);

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn journal_clean_state_does_nothing() {
        let dir = temp_dir("phase-15-5-clean-does-nothing");
        let config = config_for_dir(&dir, true);

        write_controller_journal_clean(&config.journal_path).unwrap();

        let mut executor = FakeRollbackExecutor {
            calls: 0,
            fail: false,
            affected_tasks: 31,
        };

        let outcome =
            recover_controller_journal_with_executor(config.clone(), &mut executor).unwrap();

        assert_eq!(outcome, StartupRecoveryOutcome::Clean);
        assert_eq!(executor.calls, 0);

        assert!(
            crate::autotune::controller_journal::read_controller_journal(&config.journal_path)
                .unwrap()
                .is_clean()
        );
        assert!(
            !config.audit_path.exists(),
            "clean recovery must not write an audit event"
        );
        assert!(
            !config.history_path.exists(),
            "clean recovery must not write a history event"
        );

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn rollback_failure_enters_faulted() {
        let dir = temp_dir("phase-15-5-rollback-failure-faulted");
        let config = config_for_dir(&dir, true);

        write_controller_journal_applied(
            &config.journal_path,
            "experiment-1",
            "cpu-affinity-profile:game-main",
            rollback_token(),
        )
        .unwrap();

        let mut executor = FakeRollbackExecutor {
            calls: 0,
            fail: true,
            affected_tasks: 0,
        };

        let outcome =
            recover_controller_journal_with_executor(config.clone(), &mut executor).unwrap();

        match outcome {
            StartupRecoveryOutcome::Faulted {
                experiment_id,
                action_id,
                manual_restore_command,
                reason,
            } => {
                assert_eq!(experiment_id, "experiment-1");
                assert_eq!(action_id, "cpu-affinity-profile:game-main");
                assert_eq!(manual_restore_command, "stutter restore");
                assert!(reason.contains("startup crash recovery rollback failed"));
                assert!(reason.contains("intentional recovery rollback failure"));
            }
            other => panic!("expected Faulted recovery outcome, got {other:?}"),
        }

        assert_eq!(executor.calls, 1);

        assert!(
            !crate::autotune::controller_journal::read_controller_journal(&config.journal_path)
                .unwrap()
                .is_clean(),
            "failed rollback must leave the applied journal intact for manual recovery"
        );

        let audit = crate::audit::read_audit_tail(&config.audit_path, 10).unwrap();
        assert_eq!(audit.len(), 1);
        assert!(!audit[0].success);
        assert_eq!(audit[0].command, "autotune-startup-recovery");
        assert_eq!(
            audit[0].action_id.as_deref(),
            Some("cpu-affinity-profile:game-main")
        );
        assert_eq!(audit[0].affected_tasks, 0);
        assert!(
            audit[0]
                .message
                .contains("manual_restore_command=\"stutter restore\"")
        );

        let history =
            crate::autotune::history::read_autotune_history_events(&config.history_path).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].phase, ControllerPhase::Faulted);
        assert_eq!(history[0].decision.decision, "CrashRecoveryFault");
        assert_eq!(
            history[0].action_id.as_deref(),
            Some("cpu-affinity-profile:game-main")
        );
        assert_eq!(history[0].experiment_id.as_deref(), Some("experiment-1"));
        assert!(!history[0].rollback_performed);
        assert!(
            history[0]
                .reason
                .contains("manual_restore_command=\"stutter restore\"")
        );

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn real_startup_recovery_executor_uses_universal_restore_for_sysfs_tokens() {
        let dir = temp_dir("sysfs-token");
        let path = dir.join("knob");
        fs::write(&path, "performance\n").unwrap();
        let token = RollbackToken::SysfsRestore {
            path: path.clone(),
            original_value: "powersave\n".to_owned(),
        };
        let mut executor = RealStartupRecoveryRollbackExecutor;

        let summary = executor.rollback(&token).unwrap();

        assert_eq!(summary.affected_tasks, 1);
        assert!(summary.message.contains("rollback_kind=sysfs-restore"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "powersave\n");

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn manual_restore_command_for_non_default_cpu_affinity_restore_file_copies_then_restores() {
        let token = RollbackToken::CpuAffinityRestoreFile {
            path: PathBuf::from("/tmp/custom restore.json"),
            affected_tasks: 1,
        };

        let command = manual_restore_command_for_token(&token);

        assert!(command.contains("cp --"));
        assert!(command.contains("'/tmp/custom restore.json'"));
        assert!(command.ends_with("&& stutter restore"));
    }

    #[test]
    fn manual_restore_command_supports_every_rollback_token_kind() {
        let tokens = [
            RollbackToken::CpuAffinityRestoreFile {
                path: crate::affinity::default_restore_path(),
                affected_tasks: 1,
            },
            RollbackToken::NiceRestore {
                records: vec![NiceRestoreRecord {
                    tid: 1001,
                    original_nice: 5,
                }],
            },
            RollbackToken::IrqAffinityRestore {
                records: vec![IrqAffinityRestoreRecord {
                    irq: 42,
                    device_hint: "gpu".to_owned(),
                    original_smp_affinity: "ff".to_owned(),
                }],
            },
            RollbackToken::IoPrioRestore {
                records: vec![IoPrioRestoreRecord {
                    tid: 1002,
                    original_ioprio: 0x4000,
                }],
            },
            RollbackToken::UclampRestore {
                records: vec![UclampRestoreRecord {
                    tid: 1003,
                    original_util_min: 0,
                    original_util_max: 1024,
                }],
            },
            RollbackToken::CgroupRestore {
                records: vec![CgroupRestoreRecord {
                    pid: 1004,
                    original_cgroup: PathBuf::from("/sys/fs/cgroup/game.slice"),
                }],
            },
            RollbackToken::CpuPowerRestore {
                records: vec![CpuPowerRestoreRecord {
                    path: PathBuf::from("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor"),
                    original_value: "schedutil\n".to_owned(),
                }],
            },
            RollbackToken::VmKnobRestore {
                records: vec![VmKnobRestoreRecord {
                    path: PathBuf::from("/proc/sys/vm/compaction_proactiveness"),
                    original_value: "20\n".to_owned(),
                }],
            },
            RollbackToken::GpuPowerRestore {
                records: vec![GpuPowerRestoreRecord {
                    path: PathBuf::from(
                        "/sys/class/drm/card0/device/power_dpm_force_performance_level",
                    ),
                    original_value: "auto\n".to_owned(),
                }],
            },
            RollbackToken::SysfsRestore {
                path: PathBuf::from("/sys/module/test/parameters/knob"),
                original_value: "0\n".to_owned(),
            },
        ];

        for token in tokens {
            let command = manual_restore_command_for_token(&token);

            assert!(
                !command.trim().is_empty(),
                "manual restore command must not be empty for {token:?}"
            );
            assert!(
                !command.contains("no supported manual restore command"),
                "manual restore command must be supported for {token:?}: {command}"
            );
        }
    }
}
