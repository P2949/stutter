use super::*;

#[test]
fn post_apply_transaction_phase_rolls_back_on_startup() {
    let dir = temp_dir("verifying");
    let config = config_for_dir(&dir, true);
    let record = ControllerJournalRecord::for_phase(
        ControllerJournalState::Verifying,
        crate::autotune::experiment::ExperimentId::try_new("experiment-1").unwrap(),
        crate::actions::ActionId::try_new("cpu-affinity-profile:game-main").unwrap(),
        Some(rollback_token()),
    )
    .with_verify_result("pending");
    write_controller_journal_record(&config.journal_path, &record).unwrap();
    let mut executor = FakeRollbackExecutor {
        calls: 0,
        fail: false,
        affected_tasks: 31,
    };

    let outcome = recover_controller_journal_with_executor(config.clone(), &mut executor).unwrap();

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
            crate::autotune::experiment::ExperimentId::try_new("experiment-live").unwrap(),
            crate::actions::ActionId::try_new("cpu-affinity-profile:game-main").unwrap(),
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
fn applying_journal_without_rollback_token_does_not_attempt_recovery() {
    let dir = temp_dir("applying");
    let config = config_for_dir(&dir, true);
    write_controller_journal_applying(
        &config.journal_path,
        crate::autotune::experiment::ExperimentId::try_new("experiment-1").unwrap(),
        crate::actions::ActionId::try_new("cpu-affinity-profile:game-main").unwrap(),
    )
    .unwrap();
    let mut executor = FakeRollbackExecutor::default();
    let outcome = recover_controller_journal_with_executor(config.clone(), &mut executor).unwrap();

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
        crate::autotune::experiment::ExperimentId::try_new("experiment-1").unwrap(),
        crate::actions::ActionId::try_new("cpu-affinity-profile:game-main").unwrap(),
        rollback_token(),
    )
    .unwrap();
    let mut executor = FakeRollbackExecutor {
        calls: 0,
        fail: false,
        affected_tasks: 31,
    };

    let outcome = recover_controller_journal_with_executor(config.clone(), &mut executor).unwrap();

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
        crate::autotune::experiment::ExperimentId::try_new("experiment-1").unwrap(),
        crate::actions::ActionId::try_new("cpu-affinity-profile:game-main").unwrap(),
        rollback_token(),
    )
    .unwrap();
    let mut executor = FakeRollbackExecutor::default();

    let outcome = recover_controller_journal_with_executor(config.clone(), &mut executor).unwrap();

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
        crate::autotune::experiment::ExperimentId::try_new("experiment-1").unwrap(),
        crate::actions::ActionId::try_new("cpu-affinity-profile:game-main").unwrap(),
        rollback_token(),
    )
    .unwrap();
    let mut executor = FakeRollbackExecutor {
        calls: 0,
        fail: true,
        affected_tasks: 0,
    };

    let outcome = recover_controller_journal_with_executor(config.clone(), &mut executor).unwrap();

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
        crate::autotune::experiment::ExperimentId::try_new("experiment-1").unwrap(),
        crate::actions::ActionId::try_new("cpu-affinity-profile:game-main").unwrap(),
        rollback_token(),
    )
    .unwrap();

    let mut executor = FakeRollbackExecutor {
        calls: 0,
        fail: false,
        affected_tasks: 31,
    };

    let outcome = recover_controller_journal_with_executor(config.clone(), &mut executor).unwrap();

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
fn rollback_failure_enters_faulted() {
    let dir = temp_dir("phase-15-5-rollback-failure-faulted");
    let config = config_for_dir(&dir, true);

    write_controller_journal_applied(
        &config.journal_path,
        crate::autotune::experiment::ExperimentId::try_new("experiment-1").unwrap(),
        crate::actions::ActionId::try_new("cpu-affinity-profile:game-main").unwrap(),
        rollback_token(),
    )
    .unwrap();

    let mut executor = FakeRollbackExecutor {
        calls: 0,
        fail: true,
        affected_tasks: 0,
    };

    let outcome = recover_controller_journal_with_executor(config.clone(), &mut executor).unwrap();

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
fn real_startup_recovery_executor_uses_registry_restore_for_sysfs_tokens() {
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
    assert!(summary.message.contains("restored_items=1"));
    assert!(summary.message.contains("skipped_items=0"));
    assert_eq!(fs::read_to_string(&path).unwrap(), "powersave");

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
            records: vec![NiceRestoreRecord::new(
                TaskRestoreIdentity::observed(1001, None, Some("test".to_owned()), None, None),
                5,
            )],
        },
        RollbackToken::IrqAffinityRestore {
            records: vec![IrqAffinityRestoreRecord {
                irq: 42,
                device_hint: "gpu".to_owned(),
                original_smp_affinity: "ff".to_owned(),
            }],
        },
        RollbackToken::IoPrioRestore {
            records: vec![IoPrioRestoreRecord::new(
                TaskRestoreIdentity::observed(1002, None, Some("test".to_owned()), None, None),
                0x4000,
            )],
        },
        RollbackToken::UclampRestore {
            records: vec![UclampRestoreRecord::new(
                TaskRestoreIdentity::observed(1003, None, Some("test".to_owned()), None, None),
                0,
                1024,
            )],
        },
        RollbackToken::CgroupRestore {
            records: vec![CgroupRestoreRecord::new(
                TaskRestoreIdentity::observed(1004, None, Some("test".to_owned()), None, None),
                PathBuf::from("/sys/fs/cgroup/game.slice"),
            )],
            cpuset: None,
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
