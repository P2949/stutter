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
            AutotuneDecisionSummary, AutotuneHistoryEvent, AutotuneHistoryEventInput, AutotuneMode,
            ControllerPhase, ObservationSummary, SituationKind, append_autotune_history_event,
            default_autotune_history_path,
        },
    },
    daemon::{
        DaemonPhase, DaemonState,
        state::{DaemonStateSnapshotWriter, default_daemon_state_snapshot_path},
        state_builders::{
            StartupRecoveryDaemonStateInput, daemon_state_for_startup_recovery_snapshot,
            safety_class_for_rollback_token,
        },
        store::DaemonStateStore,
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
        let result = crate::autotune::emergency_restore::default_autotune_rollback_registry()
            .restore_token(token)
            .with_context(|| {
                format!(
                    "startup crash recovery failed to restore rollback token_kind={}",
                    rollback_token_kind(token)
                )
            })?;

        Ok(startup_recovery_summary_from_rollback_result(token, result))
    }
}

fn startup_recovery_summary_from_rollback_result(
    token: &RollbackToken,
    result: crate::actions::RollbackResult,
) -> StartupRecoveryRollbackSummary {
    let skipped_items =
        result.skipped_dead + result.skipped_identity_mismatch + result.legacy_unverified;
    let mut message = format!(
        "rollback_kind={} restored_items={} skipped_items={}",
        rollback_token_kind(token),
        result.restored,
        skipped_items
    );
    if result.errors > 0 {
        message.push_str(&format!(" errors={}", result.errors));
    }
    if !result.messages.is_empty() {
        message.push_str(" messages=\"");
        message.push_str(&result.messages.join("; "));
        message.push('"');
    }

    StartupRecoveryRollbackSummary {
        affected_tasks: result.restored,
        message,
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
                startup_recovery_daemon_state(StartupRecoveryDaemonStateInput {
                    phase: DaemonPhase::Faulted,
                    decision: "applying_without_rollback",
                    reason: reason.clone(),
                    experiment_id: &experiment_id,
                    action_id: &action_id,
                    rollback_token: None,
                    rollback_available: false,
                    include_active_experiment: true,
                    faulted: true,
                    manual_restore_command: Some("stutter daemon emergency-restore"),
                }),
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
                    startup_recovery_daemon_state(StartupRecoveryDaemonStateInput {
                        phase: DaemonPhase::Faulted,
                        decision: "missing_rollback_token",
                        reason,
                        experiment_id: &experiment_id,
                        action_id: &action_id,
                        rollback_token: None,
                        rollback_available: false,
                        include_active_experiment: true,
                        faulted: true,
                        manual_restore_command: Some("stutter daemon emergency-restore"),
                    }),
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
    let experiment_id = record
        .experiment_id
        .as_ref()
        .map(|id| id.as_str().to_owned())
        .unwrap_or_else(|| format!("{state}-unknown-experiment"));

    let action_id = record
        .action_id
        .as_ref()
        .map(|id| id.as_str().to_owned())
        .unwrap_or_else(|| format!("{state}-unknown-action"));

    (experiment_id, action_id)
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
            startup_recovery_daemon_state(StartupRecoveryDaemonStateInput {
                phase: DaemonPhase::Faulted,
                decision: "rollback_disabled",
                reason,
                experiment_id: &experiment_id,
                action_id: &action_id,
                rollback_token: Some(&rollback_token),
                rollback_available: true,
                include_active_experiment: true,
                faulted: true,
                manual_restore_command: Some(&manual_restore_command),
            }),
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

            write_startup_recovery_history_event(StartupRecoveryHistoryEventInput {
                history_path: &config.history_path,
                phase: ControllerPhase::Cooldown,
                decision: "restored",
                experiment_id: &experiment_id,
                action_id: &action_id,
                rollback_token: &rollback_token,
                rollback_performed: true,
                reason: format!(
                    "startup crash recovery rollback succeeded; manual_restore_command=\"{}\"",
                    manual_restore_command
                ),
            })?;

            write_startup_recovery_daemon_state_snapshot(
                &config.state_snapshot_path,
                startup_recovery_daemon_state(StartupRecoveryDaemonStateInput {
                    phase: DaemonPhase::Cooldown,
                    decision: "restored",
                    reason: format!(
                        "startup crash recovery rollback succeeded; affected_tasks={}",
                        summary.affected_tasks
                    ),
                    experiment_id: &experiment_id,
                    action_id: &action_id,
                    rollback_token: None,
                    rollback_available: false,
                    include_active_experiment: false,
                    faulted: false,
                    manual_restore_command: Some(&manual_restore_command),
                }),
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

            write_startup_recovery_history_event(StartupRecoveryHistoryEventInput {
                history_path: &config.history_path,
                phase: ControllerPhase::Faulted,
                decision: "CrashRecoveryFault",
                experiment_id: &experiment_id,
                action_id: &action_id,
                rollback_token: &rollback_token,
                rollback_performed: false,
                reason: format!(
                    "{}; manual_restore_command=\"{}\"",
                    reason, manual_restore_command
                ),
            })?;

            write_startup_recovery_daemon_state_snapshot(
                &config.state_snapshot_path,
                startup_recovery_daemon_state(StartupRecoveryDaemonStateInput {
                    phase: DaemonPhase::Faulted,
                    decision: "faulted",
                    reason: reason.clone(),
                    experiment_id: &experiment_id,
                    action_id: &action_id,
                    rollback_token: Some(&rollback_token),
                    rollback_available: true,
                    include_active_experiment: true,
                    faulted: true,
                    manual_restore_command: Some(&manual_restore_command),
                }),
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

struct StartupRecoveryHistoryEventInput<'a> {
    history_path: &'a Path,
    phase: ControllerPhase,
    decision: &'a str,
    experiment_id: &'a str,
    action_id: &'a str,
    rollback_token: &'a RollbackToken,
    rollback_performed: bool,
    reason: String,
}

fn write_startup_recovery_history_event(
    input: StartupRecoveryHistoryEventInput<'_>,
) -> anyhow::Result<()> {
    let event = AutotuneHistoryEvent::new(AutotuneHistoryEventInput {
        controller_id: "startup-recovery".to_owned(),
        phase: input.phase,
        mode: AutotuneMode::ApplyLowRisk,
        target: None,
        situation: SituationKind::Unknown,
        observation_summary: empty_observation_summary(),
        decision: AutotuneDecisionSummary {
            decision: input.decision.to_owned(),
            candidate_name: candidate_name_from_action_id(input.action_id),
            action_kind: Some(action_kind_from_action_id(input.action_id)),
            safety_class: Some(safety_class_for_rollback_token(input.rollback_token)),
            eligible: input.rollback_performed,
            rollback_policy: "rollback-on-crash-recovery".to_owned(),
        },
        reason: input.reason,
    })
    .with_experiment_id(input.experiment_id.to_owned())
    .with_action_id(input.action_id.to_owned())
    .with_rollback_performed(input.rollback_performed);

    append_autotune_history_event(input.history_path, &event).with_context(|| {
        format!(
            "failed to write startup recovery history event to {}",
            input.history_path.display()
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

fn startup_recovery_daemon_state(input: StartupRecoveryDaemonStateInput<'_>) -> DaemonState {
    daemon_state_for_startup_recovery_snapshot(input)
}

fn empty_observation_summary() -> ObservationSummary {
    ObservationSummary {
        target_present: false,
        active_target_count: 0,
        scored_task_count: 0,
        interval_count: 0,
        scored_samples: 0,
        diagnostic_raw_score_total: 0,
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
mod tests;
