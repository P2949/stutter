use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use tokio::signal;

use super::decision_log::{
    AutotuneDecisionLabel, AutotuneModeLabel, ControllerPhaseLabel, DecisionJsonlEntry,
    DecisionJsonlEntryInput, OnlineDataQualityLabel, SituationKindLabel, append_decision_jsonl,
};
use crate::{
    actions::{ActionId, RollbackToken, SafetyClass},
    audit::{AuditEvent, append_audit_event_to_path, audit_or_warn},
    autotune::{kept::ActiveProfileState, planning::candidate::CandidateAction},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShutdownReason {
    CtrlC,
    TargetExit,
    DaemonStop,
    ControllerFault,
}

impl ShutdownReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CtrlC => "ctrl-c",
            Self::TargetExit => "target-exit",
            Self::DaemonStop => "daemon-stop",
            Self::ControllerFault => "controller-fault",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct RollbackOnExitConfig {
    pub keep_on_exit: bool,
}

#[derive(Clone, Debug)]
pub struct ActiveAutotuneAction {
    pub action_id: ActionId,
    pub action_kind: String,
    pub safety_class: SafetyClass,
    pub rollback: RollbackToken,
    pub candidate: Option<CandidateAction>,
}

impl ActiveAutotuneAction {
    pub fn cpu_affinity_profile(action_id: ActionId, rollback: RollbackToken) -> Self {
        Self {
            action_id,
            action_kind: "cpu_affinity_profile".to_owned(),
            safety_class: SafetyClass::ReversibleLowRisk,
            rollback,
            candidate: None,
        }
    }

    pub fn try_cpu_affinity_profile(
        action_id: impl Into<String>,
        rollback: RollbackToken,
    ) -> Result<Self, stutter_core::ids::EmptyStringIdError> {
        Ok(Self::cpu_affinity_profile(
            ActionId::try_new(action_id)?,
            rollback,
        ))
    }

    pub fn from_kept_candidate(kept: &crate::autotune::kept::KeptCandidateState) -> Option<Self> {
        let descriptor = kept.candidate.descriptor();
        if descriptor.persistent_effect {
            return None;
        }

        Some(Self {
            action_id: descriptor.action_id,
            action_kind: descriptor.action_kind,
            safety_class: descriptor.safety_class,
            rollback: kept.rollback.clone(),
            candidate: Some(kept.candidate.clone()),
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct ActiveAutotuneActionRegistry {
    actions: Arc<Mutex<Vec<ActiveAutotuneAction>>>,
}

impl ActiveAutotuneActionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, action: ActiveAutotuneAction) {
        self.actions
            .lock()
            // invariant: registry lock is not poisoned
            .expect("active autotune action registry poisoned")
            .push(action);
    }

    pub fn len(&self) -> usize {
        self.actions
            .lock()
            // invariant: registry lock is not poisoned
            .expect("active autotune action registry poisoned")
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn drain(&self) -> Vec<ActiveAutotuneAction> {
        self.actions
            .lock()
            // invariant: registry lock is not poisoned
            .expect("active autotune action registry poisoned")
            .drain(..)
            .collect()
    }

    pub fn snapshot(&self) -> Vec<ActiveAutotuneAction> {
        self.actions
            .lock()
            // invariant: registry lock is not poisoned
            .expect("active autotune action registry poisoned")
            .clone()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExitRollbackSummary {
    pub reason: ShutdownReason,
    pub keep_on_exit: bool,
    pub attempted_actions: usize,
    pub rolled_back_actions: usize,
    pub failed_actions: usize,
    pub skipped_actions: usize,
}

impl ExitRollbackSummary {
    pub fn success(&self) -> bool {
        self.failed_actions == 0
    }
}

pub trait ExitRollbackExecutor {
    fn rollback(&mut self, action: &ActiveAutotuneAction) -> anyhow::Result<usize>;
}

#[derive(Default)]
pub struct CpuAffinityExitRollbackExecutor;

impl ExitRollbackExecutor for CpuAffinityExitRollbackExecutor {
    fn rollback(&mut self, action: &ActiveAutotuneAction) -> anyhow::Result<usize> {
        ensure_exit_rollback_action_allowed(action)?;

        if let Some(candidate) = action.candidate.clone() {
            let executor = crate::autotune::apply::executor_for_candidate_preview(candidate)?;
            executor.rollback(&action.rollback)?;
            return Ok(action.rollback.affected_tasks());
        }

        let restore_path = action
            .rollback
            .restore_path()
            .cloned()
            .unwrap_or_else(crate::affinity::default_restore_path);

        let summary = crate::affinity::restore_saved(&restore_path).with_context(|| {
            format!(
                "failed to restore CPU affinity from {}",
                restore_path.display()
            )
        })?;

        Ok(summary.restored)
    }
}

pub fn ensure_exit_rollback_action_allowed(action: &ActiveAutotuneAction) -> anyhow::Result<()> {
    if let Some(candidate) = action.candidate.as_ref() {
        let descriptor = candidate.descriptor();
        if descriptor.persistent_effect {
            anyhow::bail!(
                "rollback-on-exit cannot restore persistent-effect actions automatically; blocked action_kind={}",
                descriptor.action_kind
            );
        }
        if action.action_kind != descriptor.action_kind {
            anyhow::bail!(
                "rollback-on-exit action kind mismatch; action_kind={} candidate_action_kind={}",
                action.action_kind,
                descriptor.action_kind
            );
        }
        if action.safety_class != descriptor.safety_class {
            anyhow::bail!(
                "rollback-on-exit safety class mismatch; safety_class={:?} candidate_safety_class={:?}",
                action.safety_class,
                descriptor.safety_class
            );
        }
        if action.safety_class > SafetyClass::ReversibleMediumRisk {
            anyhow::bail!(
                "rollback-on-exit only supports non-persistent reversible low/medium risk kept actions; blocked safety_class={:?}",
                action.safety_class
            );
        }
        return Ok(());
    }

    if action.action_kind != "cpu_affinity_profile" {
        anyhow::bail!(
            "rollback-on-exit only supports CPU affinity profile actions; blocked action_kind={}",
            action.action_kind
        );
    }

    if action.safety_class != SafetyClass::ReversibleLowRisk {
        anyhow::bail!(
            "rollback-on-exit only supports ReversibleLowRisk actions; blocked safety_class={:?}",
            action.safety_class
        );
    }

    if !matches!(
        action.rollback,
        RollbackToken::CpuAffinityRestoreFile { .. }
    ) {
        anyhow::bail!(
            "rollback-on-exit only supports cpu-affinity-restore-file rollback tokens; blocked rollback_kind={:?}",
            action.rollback
        );
    }

    Ok(())
}

pub fn rollback_active_low_risk_actions_on_exit(
    registry: &ActiveAutotuneActionRegistry,
    config: &RollbackOnExitConfig,
    reason: ShutdownReason,
    decision_log_path: Option<&Path>,
) -> ExitRollbackSummary {
    let mut executor = CpuAffinityExitRollbackExecutor;
    rollback_active_low_risk_actions_on_exit_with_executor(
        registry,
        config,
        reason,
        decision_log_path,
        &mut executor,
    )
}

pub fn rollback_active_low_risk_actions_on_exit_with_executor<E: ExitRollbackExecutor + ?Sized>(
    registry: &ActiveAutotuneActionRegistry,
    config: &RollbackOnExitConfig,
    reason: ShutdownReason,
    decision_log_path: Option<&Path>,
    executor: &mut E,
) -> ExitRollbackSummary {
    let actions = if config.keep_on_exit {
        registry.snapshot()
    } else {
        registry.drain()
    };

    let mut summary = ExitRollbackSummary {
        reason,
        keep_on_exit: config.keep_on_exit,
        attempted_actions: 0,
        rolled_back_actions: 0,
        failed_actions: 0,
        skipped_actions: 0,
    };

    if config.keep_on_exit {
        summary.skipped_actions = actions.len();
        for action in &actions {
            write_exit_audit_event(
                action,
                reason,
                true,
                true,
                0,
                "rollback skipped because keep_on_exit is true".to_owned(),
            );
        }
        write_exit_decision_event(
            decision_log_path,
            &summary,
            "keep_on_exit true; rollback skipped",
        );
        return summary;
    }

    for action in &actions {
        summary.attempted_actions += 1;

        match executor.rollback(action) {
            Ok(restored) => {
                summary.rolled_back_actions += 1;
                write_exit_audit_event(
                    action,
                    reason,
                    false,
                    true,
                    restored,
                    format!(
                        "autotune rollback-on-exit completed reason={} restored={}",
                        reason.as_str(),
                        restored
                    ),
                );
            }
            Err(err) => {
                summary.failed_actions += 1;
                write_exit_audit_event(
                    action,
                    reason,
                    false,
                    false,
                    0,
                    format!(
                        "autotune rollback-on-exit failed reason={} error={err:#}",
                        reason.as_str()
                    ),
                );
            }
        }
    }

    let decision_reason = format!(
        "shutdown reason={} attempted={} rolled_back={} failed={} skipped={}",
        reason.as_str(),
        summary.attempted_actions,
        summary.rolled_back_actions,
        summary.failed_actions,
        summary.skipped_actions
    );
    write_exit_decision_event(decision_log_path, &summary, &decision_reason);

    summary
}

#[cfg(test)]
pub fn rollback_active_low_risk_actions_on_exit_with_audit_path_for_tests<
    E: ExitRollbackExecutor + ?Sized,
>(
    registry: &ActiveAutotuneActionRegistry,
    config: &RollbackOnExitConfig,
    reason: ShutdownReason,
    decision_log_path: Option<&Path>,
    audit_path: &Path,
    executor: &mut E,
) -> ExitRollbackSummary {
    let actions = if config.keep_on_exit {
        registry.snapshot()
    } else {
        registry.drain()
    };

    let mut summary = ExitRollbackSummary {
        reason,
        keep_on_exit: config.keep_on_exit,
        attempted_actions: 0,
        rolled_back_actions: 0,
        failed_actions: 0,
        skipped_actions: 0,
    };

    if config.keep_on_exit {
        summary.skipped_actions = actions.len();
        for action in &actions {
            write_exit_audit_event_to_path(
                audit_path,
                action,
                reason,
                true,
                true,
                0,
                "rollback skipped because keep_on_exit is true".to_owned(),
            )
            // invariant: test audit write should succeed in test fixtures
            .expect("test audit write should succeed");
        }
        write_exit_decision_event(
            decision_log_path,
            &summary,
            "keep_on_exit true; rollback skipped",
        );
        return summary;
    }

    for action in &actions {
        summary.attempted_actions += 1;

        match executor.rollback(action) {
            Ok(restored) => {
                summary.rolled_back_actions += 1;
                write_exit_audit_event_to_path(
                    audit_path,
                    action,
                    reason,
                    false,
                    true,
                    restored,
                    format!(
                        "autotune rollback-on-exit completed reason={} restored={}",
                        reason.as_str(),
                        restored
                    ),
                )
                // invariant: test audit write should succeed in test fixtures
                .expect("test audit write should succeed");
            }
            Err(err) => {
                summary.failed_actions += 1;
                write_exit_audit_event_to_path(
                    audit_path,
                    action,
                    reason,
                    false,
                    false,
                    0,
                    format!(
                        "autotune rollback-on-exit failed reason={} error={err:#}",
                        reason.as_str()
                    ),
                )
                // invariant: test audit write should succeed in test fixtures
                .expect("test audit write should succeed");
            }
        }
    }

    let decision_reason = format!(
        "shutdown reason={} attempted={} rolled_back={} failed={} skipped={}",
        reason.as_str(),
        summary.attempted_actions,
        summary.rolled_back_actions,
        summary.failed_actions,
        summary.skipped_actions
    );
    write_exit_decision_event(decision_log_path, &summary, &decision_reason);

    summary
}

fn write_exit_audit_event(
    action: &ActiveAutotuneAction,
    reason: ShutdownReason,
    keep_on_exit: bool,
    success: bool,
    affected_tasks: usize,
    message: String,
) {
    audit_or_warn(&build_exit_audit_event(
        action,
        reason,
        keep_on_exit,
        success,
        affected_tasks,
        message,
    ));
}

fn write_exit_audit_event_to_path(
    path: &Path,
    action: &ActiveAutotuneAction,
    reason: ShutdownReason,
    keep_on_exit: bool,
    success: bool,
    affected_tasks: usize,
    message: String,
) -> anyhow::Result<()> {
    append_audit_event_to_path(
        path,
        &build_exit_audit_event(
            action,
            reason,
            keep_on_exit,
            success,
            affected_tasks,
            message,
        ),
    )
}

fn build_exit_audit_event(
    action: &ActiveAutotuneAction,
    reason: ShutdownReason,
    keep_on_exit: bool,
    success: bool,
    affected_tasks: usize,
    message: String,
) -> AuditEvent {
    AuditEvent {
        schema_version: 1,
        unix_nanos: crate::audit::unix_nanos_now(),
        command: "autotune rollback-on-exit".to_owned(),
        action_id: Some(action.action_id.as_str().to_owned()),
        safety_class: Some(action.safety_class.clone()),
        dry_run: false,
        success,
        affected_tasks,
        restore_path: action.rollback.restore_path().cloned(),
        action_phase: None,
        error_category: None,
        message: format!(
            "{} shutdown_reason={} keep_on_exit={}",
            message,
            reason.as_str(),
            keep_on_exit
        ),
    }
}

fn write_exit_decision_event(
    decision_log_path: Option<&Path>,
    summary: &ExitRollbackSummary,
    reason: &str,
) {
    let Some(path) = decision_log_path else {
        return;
    };

    let decision = if summary.failed_actions > 0 {
        AutotuneDecisionLabel::Fault
    } else if summary.rolled_back_actions > 0 {
        AutotuneDecisionLabel::Revert
    } else {
        AutotuneDecisionLabel::Noop
    };

    let entry = DecisionJsonlEntry::new(DecisionJsonlEntryInput {
        phase: if summary.failed_actions > 0 {
            ControllerPhaseLabel::Faulted
        } else {
            ControllerPhaseLabel::Cooldown
        },
        mode: AutotuneModeLabel::ApplyLowRisk,
        target_present: false,
        situation: SituationKindLabel::Unknown,
        diagnostic_raw_score_total: 0,
        data_quality: if summary.failed_actions > 0 {
            OnlineDataQualityLabel::Low
        } else {
            OnlineDataQualityLabel::High
        },
        decision,
        reason: reason.to_owned(),
    });

    if let Err(err) = append_decision_jsonl(path, &entry) {
        log::warn!("autotune_exit_decision_log_write_failed err={err:#}");
    }
}

pub async fn wait_for_ctrl_c_and_rollback(
    registry: ActiveAutotuneActionRegistry,
    config: RollbackOnExitConfig,
    decision_log_path: Option<PathBuf>,
) -> ExitRollbackSummary {
    match signal::ctrl_c().await {
        Ok(()) => rollback_active_low_risk_actions_on_exit(
            &registry,
            &config,
            ShutdownReason::CtrlC,
            decision_log_path.as_deref(),
        ),
        Err(err) => {
            log::warn!("ctrl_c_handler_failed err={err:#}; rolling back active autotune actions");
            rollback_active_low_risk_actions_on_exit(
                &registry,
                &config,
                ShutdownReason::ControllerFault,
                decision_log_path.as_deref(),
            )
        }
    }
}

pub fn register_cpu_affinity_rollback(
    registry: &ActiveAutotuneActionRegistry,
    action_id: impl Into<String>,
    rollback: RollbackToken,
) {
    registry.register(ActiveAutotuneAction::cpu_affinity_profile(
        ActionId::new(action_id),
        rollback,
    ));
}

pub fn register_kept_actions_for_exit_rollback(
    registry: &ActiveAutotuneActionRegistry,
    active_profile_state: &ActiveProfileState,
) {
    for kept in active_profile_state.kept_actions.values() {
        if let Some(action) = ActiveAutotuneAction::from_kept_candidate(kept) {
            registry.register(action);
        }
    }
}

#[cfg(test)]
mod tests;
