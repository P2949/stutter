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
