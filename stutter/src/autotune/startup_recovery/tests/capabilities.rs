use super::*;
use crate::actions::rollback::recovery::{AutomaticRecoveryCapability, recovery_descriptor};

/// The tokens whose provider has passed its fault matrix and may be restored at startup.
fn automatically_recoverable(token: &RollbackToken) -> bool {
    matches!(
        token,
        RollbackToken::CpuAffinityRestoreFile { .. }
            | RollbackToken::UclampRestore { .. }
            | RollbackToken::NiceRestore { .. }
            | RollbackToken::CgroupRestore { .. }
    )
}

fn token_inventory() -> Vec<RollbackToken> {
    vec![
        RollbackToken::CpuAffinityRestoreFile {
            path: PathBuf::from("unused-restore.json"),
            affected_tasks: 1,
        },
        RollbackToken::NiceRestore { records: vec![] },
        RollbackToken::IoPrioRestore { records: vec![] },
        RollbackToken::UclampRestore { records: vec![] },
        RollbackToken::CgroupRestore {
            records: vec![],
            cpuset: None,
        },
        RollbackToken::IrqAffinityRestore { records: vec![] },
        RollbackToken::CpuPowerRestore { records: vec![] },
        RollbackToken::GpuPowerRestore { records: vec![] },
        RollbackToken::VmKnobRestore { records: vec![] },
        RollbackToken::SysfsRestore {
            path: PathBuf::from("unused-knob"),
            original_value: "0".to_owned(),
        },
    ]
}

#[test]
fn startup_registry_has_only_explicitly_authorized_providers_and_no_manual_fallback() {
    let registry = crate::autotune::emergency_restore::startup_rollback_registry();
    assert_eq!(registry.handlers().len(), 4);
    let mut kinds = std::collections::BTreeSet::new();
    for token in token_inventory() {
        let descriptor = recovery_descriptor(&token);
        assert!(kinds.insert(descriptor.token_kind));
        assert!(!descriptor.identity_contract.is_empty());
        let automatic = automatically_recoverable(&token);
        assert_eq!(
            descriptor.automatic_recovery == AutomaticRecoveryCapability::VerifiedAutomatic,
            automatic
        );
        if !automatic {
            let expected = if matches!(token, RollbackToken::SysfsRestore { .. }) {
                AutomaticRecoveryCapability::NotRecoverable
            } else {
                AutomaticRecoveryCapability::ManualOnly
            };
            assert_eq!(descriptor.automatic_recovery, expected, "{}", token.kind());
        }
        let supported = registry
            .handlers()
            .iter()
            .any(|handler| handler.supports_token(&token));
        assert_eq!(supported, automatic, "{}", token.kind());
        if !automatic {
            assert!(
                RealStartupRecoveryRollbackExecutor::default()
                    .rollback(&token)
                    .is_err()
            );
        }
    }
    assert_eq!(kinds.len(), 10);
}

#[test]
fn unproven_provider_faults_before_executor_dispatch_and_retains_recovery_evidence() {
    for token in token_inventory()
        .into_iter()
        .filter(|token| !automatically_recoverable(token))
    {
        let root = crate::test_support::TestRoot::new(token.kind());
        let config = config_for_dir(root.path(), true);
        write_controller_journal_applied(
            &config.journal_path,
            experiment_id("unproven-provider"),
            action_id("unproven-action"),
            token.clone(),
        )
        .unwrap();
        let mut executor = FakeRollbackExecutor {
            affected_tasks: 1,
            ..Default::default()
        };
        let outcome =
            recover_controller_journal_with_executor(config.clone(), &mut executor).unwrap();
        match outcome {
            StartupRecoveryOutcome::Faulted {
                reason,
                manual_restore_command,
                ..
            } => {
                assert!(
                    reason.contains("startup_recovery_not_authorized"),
                    "{reason}"
                );
                assert!(!manual_restore_command.is_empty());
            }
            other => panic!("unproven provider must fault: {other:?}"),
        }
        assert_eq!(executor.calls, 0);
        let journal =
            crate::autotune::controller_journal::read_controller_journal(&config.journal_path)
                .unwrap();
        assert_eq!(journal.rollback_token(), Some(&token));
        assert!(!journal.is_clean());
        let audit = crate::audit::read_audit_tail(&config.audit_path, 10).unwrap();
        assert_eq!(audit.len(), 1);
        assert!(!audit[0].success);
        assert!(audit[0].message.contains("startup_recovery_not_authorized"));
        let history =
            crate::autotune::history::read_autotune_history_events(&config.history_path).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].phase, ControllerPhase::Faulted);
        assert!(!history[0].rollback_performed);
        assert_eq!(
            read_daemon_state_snapshot(&config.state_snapshot_path).phase,
            DaemonPhase::Faulted
        );
    }
}

#[test]
fn explicit_manual_sysfs_restore_remains_separate_from_startup_authorization() {
    let root = crate::test_support::TestRoot::new("manual-sysfs-compatibility");
    let path = root.join("knob");
    fs::write(&path, "performance\n").unwrap();
    let token = RollbackToken::SysfsRestore {
        path: path.clone(),
        original_value: "powersave".to_owned(),
    };
    let summary = crate::autotune::emergency_restore::restore_rollback_token(&token).unwrap();
    assert_eq!(summary.restored_items, 1);
    assert_eq!(fs::read_to_string(path).unwrap(), "powersave");
}

#[test]
fn a_nice_token_is_restored_through_the_privileged_worker_and_never_in_process_when_one_is_named() {
    use crate::autotune::startup_recovery::StartupRecoveryRollbackExecutor;
    let token = RollbackToken::NiceRestore { records: vec![] };
    assert!(privileged::needs_privileged_restore(&token));
    assert!(!privileged::needs_privileged_restore(
        &RollbackToken::UclampRestore { records: vec![] }
    ));
    // With a worker named, the restore goes there; an unreachable worker is a failure that keeps
    // the token (it must not fall back to an in-process restore the user cannot perform).
    let root = crate::test_support::TestRoot::new("nice-privileged-route");
    let mut executor = RealStartupRecoveryRollbackExecutor {
        privileged_worker_socket: Some(root.join("missing.sock")),
    };
    let error = executor.rollback(&token).unwrap_err();
    assert!(
        format!("{error:#}").contains("privileged worker"),
        "{error:#}"
    );
}
