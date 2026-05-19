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
    actions::{RollbackToken, SafetyClass},
    audit::{AuditEvent, append_audit_event_to_path, audit_or_warn},
    autotune::{candidate::CandidateAction, kept::ActiveProfileState},
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
    pub action_id: String,
    pub action_kind: String,
    pub safety_class: SafetyClass,
    pub rollback: RollbackToken,
    pub candidate: Option<CandidateAction>,
}

impl ActiveAutotuneAction {
    pub fn cpu_affinity_profile(action_id: impl Into<String>, rollback: RollbackToken) -> Self {
        Self {
            action_id: action_id.into(),
            action_kind: "cpu_affinity_profile".to_owned(),
            safety_class: SafetyClass::ReversibleLowRisk,
            rollback,
            candidate: None,
        }
    }

    pub fn from_kept_candidate(kept: &crate::autotune::kept::KeptCandidateState) -> Option<Self> {
        let descriptor = kept.candidate.descriptor();
        if descriptor.persistent_effect {
            return None;
        }

        Some(Self {
            action_id: descriptor.action_id.0,
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
            .expect("active autotune action registry poisoned")
            .push(action);
    }

    pub fn len(&self) -> usize {
        self.actions
            .lock()
            .expect("active autotune action registry poisoned")
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn drain(&self) -> Vec<ActiveAutotuneAction> {
        self.actions
            .lock()
            .expect("active autotune action registry poisoned")
            .drain(..)
            .collect()
    }

    pub fn snapshot(&self) -> Vec<ActiveAutotuneAction> {
        self.actions
            .lock()
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
        action_id: Some(action.action_id.clone()),
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
        score_total: 0,
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
        action_id, rollback,
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
mod tests {
    use std::fs;

    use super::*;
    use crate::{
        affinity::CpuMask,
        autotune::{comparison::ExperimentResult, experiment::WindowScore},
        process_tree::TaskClass,
        profiles::{Profile, ProfileRule},
        scorer::StutterScore,
    };

    #[derive(Default)]
    struct FakeExecutor {
        calls: usize,
        fail: bool,
        restored: usize,
    }

    impl ExitRollbackExecutor for FakeExecutor {
        fn rollback(&mut self, action: &ActiveAutotuneAction) -> anyhow::Result<usize> {
            self.calls += 1;
            ensure_exit_rollback_action_allowed(action)?;

            if self.fail {
                anyhow::bail!("intentional rollback failure");
            }

            Ok(self.restored)
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-autotune-shutdown-test-{name}-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn rollback_token() -> RollbackToken {
        RollbackToken::CpuAffinityRestoreFile {
            path: PathBuf::from("/tmp/stutter-restore.json"),
            affected_tasks: 31,
        }
    }

    fn action(name: &str) -> ActiveAutotuneAction {
        ActiveAutotuneAction::cpu_affinity_profile(name, rollback_token())
    }

    fn profile(name: &str) -> Profile {
        Profile {
            name: name.to_owned(),
            rules: vec![ProfileRule {
                affinity: Some(CpuMask::parse("0").unwrap()),
                nice: None,
                ionice: None,
                match_class: vec![TaskClass::Game],
                match_comm: Vec::new(),
            }],
        }
    }

    fn window_score(total: u64) -> WindowScore {
        WindowScore {
            started_unix_nanos: 1,
            finished_unix_nanos: 2,
            interval_count: 10,
            scored_samples: 100,
            scored_task_count: 1,
            score: StutterScore {
                total,
                ..StutterScore::default()
            },
        }
    }

    #[test]
    fn default_config_rolls_back_on_exit() {
        assert!(!RollbackOnExitConfig::default().keep_on_exit);
    }

    #[test]
    fn ctrl_c_rolls_back_all_active_actions_and_drains_registry() {
        let dir = temp_dir("ctrl-c");
        let audit_path = dir.join("audit.jsonl");
        let decision_path = dir.join("decisions.jsonl");
        let registry = ActiveAutotuneActionRegistry::new();
        registry.register(action("cpu-affinity-profile:game-main"));
        registry.register(action("cpu-affinity-profile:game-helper"));
        let mut executor = FakeExecutor {
            calls: 0,
            fail: false,
            restored: 31,
        };

        let summary = rollback_active_low_risk_actions_on_exit_with_audit_path_for_tests(
            &registry,
            &RollbackOnExitConfig::default(),
            ShutdownReason::CtrlC,
            Some(&decision_path),
            &audit_path,
            &mut executor,
        );

        assert_eq!(summary.reason, ShutdownReason::CtrlC);
        assert_eq!(summary.attempted_actions, 2);
        assert_eq!(summary.rolled_back_actions, 2);
        assert_eq!(summary.failed_actions, 0);
        assert_eq!(summary.skipped_actions, 0);
        assert!(summary.success());
        assert_eq!(executor.calls, 2);
        assert!(registry.is_empty());

        let audit_events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(audit_events.len(), 2);
        assert!(
            audit_events
                .iter()
                .all(|event| event.command == "autotune rollback-on-exit")
        );
        assert!(audit_events.iter().all(|event| event.success));
        assert!(
            audit_events
                .iter()
                .all(|event| event.safety_class == Some(SafetyClass::ReversibleLowRisk))
        );
        assert!(
            audit_events
                .iter()
                .all(|event| event.message.contains("shutdown_reason=ctrl-c"))
        );

        let decisions = crate::autotune::decision_log::read_decision_jsonl(&decision_path).unwrap();
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].decision, AutotuneDecisionLabel::Revert);
        assert!(decisions[0].reason.contains("shutdown reason=ctrl-c"));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn default_exit_rollback_executor_restores_empty_restore_file() {
        let dir = temp_dir("default-executor");
        let restore_path = dir.join("restore.json");
        crate::affinity::save_restore_state(&restore_path, &[]).unwrap();

        let registry = ActiveAutotuneActionRegistry::new();
        register_cpu_affinity_rollback(
            &registry,
            "cpu-affinity-profile:empty-restore",
            RollbackToken::CpuAffinityRestoreFile {
                path: restore_path.clone(),
                affected_tasks: 0,
            },
        );

        let summary = rollback_active_low_risk_actions_on_exit(
            &registry,
            &RollbackOnExitConfig::default(),
            ShutdownReason::DaemonStop,
            None,
        );

        assert_eq!(summary.reason, ShutdownReason::DaemonStop);
        assert_eq!(summary.attempted_actions, 1);
        assert_eq!(summary.rolled_back_actions, 1);
        assert_eq!(summary.failed_actions, 0);
        assert_eq!(summary.skipped_actions, 0);
        assert!(summary.success());
        assert!(registry.is_empty());
        assert!(!restore_path.exists());

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn ctrl_c_rollback_handler_future_is_constructible_without_polling_signal() {
        let _future = wait_for_ctrl_c_and_rollback(
            ActiveAutotuneActionRegistry::new(),
            RollbackOnExitConfig::default(),
            None,
        );
    }

    #[test]
    fn target_exit_rolls_back_active_actions() {
        let registry = ActiveAutotuneActionRegistry::new();
        registry.register(action("cpu-affinity-profile:game-main"));
        let mut executor = FakeExecutor {
            calls: 0,
            fail: false,
            restored: 31,
        };

        let summary = rollback_active_low_risk_actions_on_exit_with_executor(
            &registry,
            &RollbackOnExitConfig::default(),
            ShutdownReason::TargetExit,
            None,
            &mut executor,
        );

        assert_eq!(summary.reason, ShutdownReason::TargetExit);
        assert_eq!(summary.rolled_back_actions, 1);
        assert_eq!(executor.calls, 1);
        assert!(registry.is_empty());
    }

    #[test]
    fn daemon_stop_rolls_back_active_actions() {
        let registry = ActiveAutotuneActionRegistry::new();
        registry.register(action("cpu-affinity-profile:game-main"));
        let mut executor = FakeExecutor {
            calls: 0,
            fail: false,
            restored: 31,
        };

        let summary = rollback_active_low_risk_actions_on_exit_with_executor(
            &registry,
            &RollbackOnExitConfig::default(),
            ShutdownReason::DaemonStop,
            None,
            &mut executor,
        );

        assert_eq!(summary.reason, ShutdownReason::DaemonStop);
        assert_eq!(summary.rolled_back_actions, 1);
        assert_eq!(executor.calls, 1);
        assert!(registry.is_empty());
    }

    #[test]
    fn controller_fault_rolls_back_and_writes_fault_decision_on_failure() {
        let dir = temp_dir("controller-fault");
        let audit_path = dir.join("audit.jsonl");
        let decision_path = dir.join("decisions.jsonl");
        let registry = ActiveAutotuneActionRegistry::new();
        registry.register(action("cpu-affinity-profile:game-main"));
        let mut executor = FakeExecutor {
            calls: 0,
            fail: true,
            restored: 0,
        };

        let summary = rollback_active_low_risk_actions_on_exit_with_audit_path_for_tests(
            &registry,
            &RollbackOnExitConfig::default(),
            ShutdownReason::ControllerFault,
            Some(&decision_path),
            &audit_path,
            &mut executor,
        );

        assert_eq!(summary.reason, ShutdownReason::ControllerFault);
        assert_eq!(summary.attempted_actions, 1);
        assert_eq!(summary.rolled_back_actions, 0);
        assert_eq!(summary.failed_actions, 1);
        assert!(!summary.success());
        assert_eq!(executor.calls, 1);
        assert!(registry.is_empty());

        let audit_events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(audit_events.len(), 1);
        assert!(!audit_events[0].success);
        assert!(audit_events[0].message.contains("rollback-on-exit failed"));
        assert!(
            audit_events[0]
                .message
                .contains("shutdown_reason=controller-fault")
        );

        let decisions = crate::autotune::decision_log::read_decision_jsonl(&decision_path).unwrap();
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].phase, ControllerPhaseLabel::Faulted);
        assert_eq!(decisions[0].decision, AutotuneDecisionLabel::Fault);

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn keep_on_exit_skips_rollback_and_preserves_registry() {
        let dir = temp_dir("keep");
        let audit_path = dir.join("audit.jsonl");
        let decision_path = dir.join("decisions.jsonl");
        let registry = ActiveAutotuneActionRegistry::new();
        registry.register(action("cpu-affinity-profile:game-main"));
        let mut executor = FakeExecutor {
            calls: 0,
            fail: false,
            restored: 31,
        };

        let summary = rollback_active_low_risk_actions_on_exit_with_audit_path_for_tests(
            &registry,
            &RollbackOnExitConfig { keep_on_exit: true },
            ShutdownReason::CtrlC,
            Some(&decision_path),
            &audit_path,
            &mut executor,
        );

        assert_eq!(summary.attempted_actions, 0);
        assert_eq!(summary.rolled_back_actions, 0);
        assert_eq!(summary.failed_actions, 0);
        assert_eq!(summary.skipped_actions, 1);
        assert_eq!(executor.calls, 0);
        assert_eq!(registry.len(), 1);

        let audit_events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(audit_events.len(), 1);
        assert!(audit_events[0].success);
        assert!(
            audit_events[0]
                .message
                .contains("rollback skipped because keep_on_exit is true")
        );

        let decisions = crate::autotune::decision_log::read_decision_jsonl(&decision_path).unwrap();
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].decision, AutotuneDecisionLabel::Noop);
        assert!(decisions[0].reason.contains("keep_on_exit true"));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn non_low_risk_action_is_blocked() {
        let mut bad = action("cpu-affinity-profile:game-main");
        bad.safety_class = SafetyClass::HighRisk;

        let err = ensure_exit_rollback_action_allowed(&bad)
            .unwrap_err()
            .to_string();

        assert!(err.contains("only supports ReversibleLowRisk"));
    }

    #[test]
    fn non_cpu_affinity_action_is_blocked() {
        let mut bad = action("gpu-power-profile");
        bad.action_kind = "gpu_power_profile".to_owned();

        let err = ensure_exit_rollback_action_allowed(&bad)
            .unwrap_err()
            .to_string();

        assert!(err.contains("only supports CPU affinity profile actions"));
    }

    #[test]
    fn non_cpu_affinity_rollback_token_is_blocked() {
        let mut bad = action("cpu-affinity-profile:game-main");
        bad.rollback = RollbackToken::SysfsRestore {
            path: PathBuf::from("/sys/class/drm/card0/device/power_dpm_force_performance_level"),
            original_value: "auto".to_owned(),
        };

        let err = ensure_exit_rollback_action_allowed(&bad)
            .unwrap_err()
            .to_string();

        assert!(err.contains("only supports cpu-affinity-restore-file rollback tokens"));
    }

    #[test]
    fn register_cpu_affinity_rollback_adds_active_action() {
        let registry = ActiveAutotuneActionRegistry::new();

        register_cpu_affinity_rollback(
            &registry,
            "cpu-affinity-profile:game-main",
            rollback_token(),
        );

        let actions = registry.snapshot();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].action_id, "cpu-affinity-profile:game-main");
        assert_eq!(actions[0].action_kind, "cpu_affinity_profile");
        assert_eq!(actions[0].safety_class, SafetyClass::ReversibleLowRisk);
    }

    #[test]
    fn register_kept_actions_for_exit_rollback_adds_non_persistent_kept_actions() {
        let registry = ActiveAutotuneActionRegistry::new();
        let mut active_profile_state = ActiveProfileState::default();
        let candidate = CandidateAction::cpu_affinity_profile(profile("game-main"), 1234);
        let kept = crate::autotune::kept::KeptCandidateState::new(
            crate::autotune::experiment::ExperimentId::new("experiment-1"),
            candidate,
            window_score(1_000),
            window_score(800),
            rollback_token(),
            123,
            "kept for shutdown restore",
        );
        active_profile_state
            .record_kept_candidate(
                kept,
                ExperimentResult::Improved {
                    improvement_percent: 20.0,
                },
            )
            .unwrap();

        register_kept_actions_for_exit_rollback(&registry, &active_profile_state);

        let actions = registry.snapshot();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].action_kind, "cpu_affinity_profile");
        assert!(actions[0].candidate.is_some());
        assert_eq!(actions[0].rollback.affected_tasks(), 31);
    }
}
