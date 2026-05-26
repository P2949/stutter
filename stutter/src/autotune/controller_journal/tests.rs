    use super::*;
    use crate::{
        actions::{RollbackToken, SafetyClass},
        daemon::policy::DaemonMode,
    };
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    fn temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-controller-journal-test-{name}-{}-{}",
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

    fn temporary_files_in(dir: &Path) -> Vec<PathBuf> {
        fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("tmp"))
            .collect()
    }

    #[test]
    fn process_identity_label_includes_starttime_and_active_tasks_when_known() {
        assert_eq!(
            journal_process_identity(1234, Some(99), Some(7)),
            "pid:1234:starttime:99:active_tasks:7"
        );
        assert_eq!(
            journal_process_identity(1234, None, None),
            "pid:1234:starttime:unknown"
        );
    }

    #[test]
    fn applying_journal_serializes_without_rollback_token_until_applied() {
        let record =
            ControllerJournalRecord::applying("experiment-1", "cpu-affinity-profile:game-main");

        let value = serde_json::to_value(&record).unwrap();

        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["state"], "applying");
        assert_eq!(value["experiment_id"], "experiment-1");
        assert_eq!(value["action_id"], "cpu-affinity-profile:game-main");
        assert!(value.get("rollback_token").is_none());
        assert_eq!(record.state(), ControllerJournalState::Applying);
        assert_eq!(
            record.experiment_action(),
            Some(("experiment-1", "cpu-affinity-profile:game-main"))
        );
        assert!(record.is_active_experiment_state());
    }

    #[test]
    fn applied_journal_serializes_with_rollback_token() {
        let record = ControllerJournalRecord::applied(
            "experiment-1",
            "cpu-affinity-profile:game-main",
            rollback_token(),
        );

        let value = serde_json::to_value(&record).unwrap();

        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["state"], "applied");
        assert_eq!(value["experiment_id"], "experiment-1");
        assert_eq!(value["action_id"], "cpu-affinity-profile:game-main");
        assert_eq!(value["rollback_token"]["kind"], "cpu-affinity-restore-file");
        assert_eq!(value["rollback_token"]["affected_tasks"], 31);
        assert_eq!(record.state(), ControllerJournalState::Applied);
        assert!(record.rollback_token().is_some());
        assert!(record.may_have_mutated_system());
    }

    #[test]
    fn clean_journal_serializes_without_action_fields() {
        let record = ControllerJournalRecord::clean();

        let value = serde_json::to_value(&record).unwrap();

        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["state"], "clean");
        assert!(value.get("experiment_id").is_none());
        assert!(value.get("action_id").is_none());
        assert!(value.get("rollback_token").is_none());
        assert!(record.is_clean());
        assert!(!record.is_active_experiment_state());
    }

    #[test]
    fn transaction_journal_supports_expanded_phases_and_metadata() {
        let rollback = rollback_token();
        let phases = [
            ControllerJournalState::Planned,
            ControllerJournalState::Preflighted,
            ControllerJournalState::Applying,
            ControllerJournalState::Applied,
            ControllerJournalState::Verifying,
            ControllerJournalState::Measuring,
            ControllerJournalState::Keeping,
            ControllerJournalState::Reverting,
            ControllerJournalState::Reverted,
            ControllerJournalState::Faulted,
        ];

        for phase in phases {
            let record = ControllerJournalRecord::for_phase(
                phase,
                "experiment-1",
                "cpu-affinity-profile:game-main",
                matches!(
                    phase,
                    ControllerJournalState::Applied
                        | ControllerJournalState::Verifying
                        | ControllerJournalState::Measuring
                        | ControllerJournalState::Keeping
                        | ControllerJournalState::Reverting
                        | ControllerJournalState::Faulted
                )
                .then(|| rollback.clone()),
            )
            .with_candidate("game-main")
            .with_workload_identity("workload:game")
            .with_target_identity("pid:1234:start:99")
            .with_restore_command("stutter daemon emergency-restore")
            .with_verify_result("pending")
            .with_mode(DaemonMode::ApplyLowRisk)
            .with_safety_class(SafetyClass::ReversibleLowRisk);

            let value = serde_json::to_value(&record).unwrap();

            assert_eq!(value["schema_version"], 1);
            assert_eq!(record.state(), phase);
            assert_eq!(record.candidate.as_deref(), Some("game-main"));
            assert_eq!(record.workload_identity.as_deref(), Some("workload:game"));
            assert_eq!(record.target_identity.as_deref(), Some("pid:1234:start:99"));
            assert_eq!(
                record.restore_command.as_deref(),
                Some("stutter daemon emergency-restore")
            );
            assert_eq!(record.verify_result.as_deref(), Some("pending"));
            assert_eq!(record.mode, Some(DaemonMode::ApplyLowRisk));
            assert_eq!(record.safety_class, Some(SafetyClass::ReversibleLowRisk));
        }
    }

    #[test]
    fn journal_round_trips_and_atomic_write_removes_temp_file() {
        let dir = temp_dir("round-trip");
        let path = dir.join("controller_journal.json");

        let applying = write_controller_journal_applying(
            &path,
            "experiment-1",
            "cpu-affinity-profile:game-main",
        )
        .unwrap();
        assert_eq!(read_controller_journal(&path).unwrap(), applying);

        let applied = write_controller_journal_applied(
            &path,
            "experiment-1",
            "cpu-affinity-profile:game-main",
            rollback_token(),
        )
        .unwrap();
        assert_eq!(read_controller_journal(&path).unwrap(), applied);

        let clean = write_controller_journal_clean(&path).unwrap();
        assert_eq!(read_controller_journal(&path).unwrap(), clean);
        assert!(temporary_files_in(&dir).is_empty());

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn journal_writer_does_not_use_fixed_temp_path() {
        let dir = temp_dir("fixed-temp-sentinel");
        let path = dir.join("controller_journal.json");
        let fixed_temp_path = dir.join("controller_journal.json.tmp");
        fs::write(&fixed_temp_path, "sentinel").unwrap();

        write_controller_journal_applied(
            &path,
            "experiment-1",
            "cpu-affinity-profile:game-main",
            rollback_token(),
        )
        .unwrap();

        assert_eq!(fs::read_to_string(&fixed_temp_path).unwrap(), "sentinel");
        assert_eq!(
            read_controller_journal(&path).unwrap().state(),
            ControllerJournalState::Applied
        );

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn metadata_aware_writer_persists_action_context() {
        let dir = temp_dir("metadata-writer");
        let path = dir.join("controller_journal.json");
        let metadata = ControllerJournalActionMetadata::default()
            .with_candidate("game-main")
            .with_workload_identity("pid:1234:starttime:99")
            .with_target_identity("pid:1234:starttime:99:active_tasks:31")
            .with_restore_command("stutter autotune restore")
            .with_verify_result("applied_pending_verify")
            .with_mode(DaemonMode::ApplyLowRisk)
            .with_safety_class(SafetyClass::ReversibleLowRisk);

        let written = write_controller_journal_applied_with_metadata(
            &path,
            "experiment-1",
            "cpu-affinity-profile:game-main",
            rollback_token(),
            metadata,
        )
        .unwrap();
        let read = read_controller_journal(&path).unwrap();

        assert_eq!(read, written);
        assert_eq!(read.candidate.as_deref(), Some("game-main"));
        assert_eq!(
            read.workload_identity.as_deref(),
            Some("pid:1234:starttime:99")
        );
        assert_eq!(
            read.target_identity.as_deref(),
            Some("pid:1234:starttime:99:active_tasks:31")
        );
        assert_eq!(
            read.restore_command.as_deref(),
            Some("stutter autotune restore")
        );
        assert_eq!(
            read.verify_result.as_deref(),
            Some("applied_pending_verify")
        );
        assert_eq!(read.mode, Some(DaemonMode::ApplyLowRisk));
        assert_eq!(read.safety_class, Some(SafetyClass::ReversibleLowRisk));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn missing_journal_reads_as_clean() {
        let dir = temp_dir("missing");
        let path = dir.join("missing-controller-journal.json");

        let record = read_controller_journal(&path).unwrap();

        assert_eq!(record, ControllerJournalRecord::clean());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn default_controller_journal_path_matches_autotune_state_directory() {
        let path = default_controller_journal_path();
        let rendered = path.to_string_lossy();

        assert!(rendered.ends_with(".local/state/stutter/autotune/controller_journal.json"));
    }
