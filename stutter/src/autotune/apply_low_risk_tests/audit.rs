//! Audit and controller-journal tests extracted from `autotune::apply_low_risk`.
//!
//! Owns audit metadata, audit-path failure coverage, history events, controller-journal hooks, and audited runner event tests.
//! Does not own policy gates, target resolution, experiment resolution, rollback orchestration, or production behavior.

#[cfg(test)]
mod tests {
    use std::fs;

    use super::super::*;
    use crate::{
        actions::{
            SafetyClass, cpu_affinity::CpuAffinityProfileAction,
            runner::run_audited_action_with_audit_path,
        },
        profiles::Profile,
    };

    #[test]
    fn controller_journal_metadata_for_cpu_affinity_action_describes_target_and_restore() {
        let action = CpuAffinityProfileAction {
            tree_pid: 0,
            profile: Profile {
                name: "game-main".to_owned(),
                rules: Vec::new(),
            },
            force_restore_overwrite: false,
        };

        let metadata = controller_journal_metadata_for_cpu_affinity_action(
            "game-main",
            &action,
            Some(31),
            "applied_pending_verify",
        );

        assert_eq!(metadata.candidate.as_deref(), Some("game-main"));
        assert_eq!(
            metadata.workload_identity.as_deref(),
            Some("pid:0:starttime:unknown")
        );
        assert_eq!(
            metadata.target_identity.as_deref(),
            Some("pid:0:starttime:unknown:active_tasks:31")
        );
        assert_eq!(
            metadata.restore_command.as_deref(),
            Some("stutter autotune restore")
        );
        assert_eq!(
            metadata.verify_result.as_deref(),
            Some("applied_pending_verify")
        );
        assert_eq!(metadata.safety_class, Some(SafetyClass::ReversibleLowRisk));
    }

    #[test]
    fn audit_path_helper_routes_cpu_affinity_preflight_errors_through_runner() {
        let dir = temp_dir("audit-path-helper-preflight");
        let audit_path = dir.join("audit.jsonl");
        let action = test_cpu_affinity_profile_action();

        let err = apply_cpu_affinity_candidate_with_audit_path_for_tests(
            "game-main".to_owned(),
            &action,
            &audit_path,
        )
        .unwrap_err();
        let err = format!("{err:#}");

        assert!(err.contains("tree pid must be greater than zero"));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn low_risk_history_event_helper_writes_jsonl() {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-autotune-low-risk-history-test-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.jsonl");

        let score = crate::autotune::experiment::WindowScore {
            started_unix_nanos: 100,
            finished_unix_nanos: 200,
            interval_count: 10,
            scored_samples: 100,
            scored_task_count: 2,
            score: crate::scorer::StutterScore {
                total: 143,
                ..crate::scorer::StutterScore::default()
            },
        };

        let event = crate::autotune::history::AutotuneHistoryEvent::new(
            crate::autotune::history::AutotuneHistoryEventInput {
                controller_id: "controller-1".to_owned(),
                phase: crate::autotune::history::ControllerPhase::Cooldown,
                mode: crate::autotune::history::AutotuneMode::ApplyLowRisk,
                target: None,
                situation: crate::autotune::history::SituationKind::GameCpuSchedulerPressure,
                observation_summary:
                    crate::autotune::history::observation_summary_from_window_score(
                        true, 31, 0, "High", &score,
                    ),
                decision: crate::autotune::history::AutotuneDecisionSummary {
                    decision: "Revert".to_owned(),
                    candidate_name: Some("game-main".to_owned()),
                    action_kind: Some("cpu_affinity_profile".to_owned()),
                    safety_class: Some(SafetyClass::ReversibleLowRisk),
                    eligible: true,
                    rollback_policy: "rollback-on-exit".to_owned(),
                },
                reason: "regressed; rollback performed".to_owned(),
            },
        )
        .with_rollback_performed(true);

        append_low_risk_history_event(&path, &event).unwrap();

        let events = crate::autotune::history::read_autotune_history_events(&path).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], event);
        assert!(events[0].rollback_performed);

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn controller_journal_hooks_write_applied_journal_after_apply_success() {
        let dir = temp_dir("applied-journal-hook-success");
        let journal_path = dir.join("controller_journal.json");
        let experiment_id = "test-experiment";
        let action_id = "test-action";
        let profile_action = test_cpu_affinity_profile_action();

        let action = TestAction {
            id: "test-candidate",
            safety_class: SafetyClass::ReversibleLowRisk,
            should_fail_apply: false,
            should_fail_verify: false,
            affected_tasks: 31,
        };

        crate::actions::runner::run_audited_action_with_hooks(
            "test-cmd",
            &action,
            apply_policy(),
            controller_journal_hooks_for_low_risk_action(
                &journal_path,
                experiment_id,
                action_id,
                "game-main",
                &profile_action,
            ),
        )
        .unwrap();

        let record =
            crate::autotune::controller_journal::read_controller_journal(&journal_path).unwrap();
        assert_eq!(
            record.state(),
            crate::autotune::controller_journal::ControllerJournalState::Applied
        );
        assert_eq!(record.rollback_token.as_ref().unwrap().affected_tasks(), 31);
        assert_eq!(record.candidate.as_deref(), Some("game-main"));
        assert_eq!(
            record.workload_identity.as_deref(),
            Some("pid:0:starttime:unknown")
        );
        assert_eq!(
            record.target_identity.as_deref(),
            Some("pid:0:starttime:unknown:active_tasks:31")
        );
        assert_eq!(
            record.restore_command.as_deref(),
            Some("stutter autotune restore")
        );
        assert_eq!(
            record.verify_result.as_deref(),
            Some("applied_pending_verify")
        );
        assert_eq!(record.safety_class, Some(SafetyClass::ReversibleLowRisk));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn controller_journal_hooks_clean_journal_after_verify_failure_rollback() {
        let dir = temp_dir("applied-journal-hook-clean-verify-fail");
        let journal_path = dir.join("controller_journal.json");
        let experiment_id = "test-experiment";
        let action_id = "test-action";
        let profile_action = test_cpu_affinity_profile_action();

        let action = TestAction {
            id: "test-candidate",
            safety_class: SafetyClass::ReversibleLowRisk,
            should_fail_apply: false,
            should_fail_verify: true,
            affected_tasks: 31,
        };

        let result = crate::actions::runner::run_audited_action_with_hooks(
            "test-cmd",
            &action,
            apply_policy(),
            controller_journal_hooks_for_low_risk_action(
                &journal_path,
                experiment_id,
                action_id,
                "game-main",
                &profile_action,
            ),
        );

        assert!(result.is_err());
        let record =
            crate::autotune::controller_journal::read_controller_journal(&journal_path).unwrap();
        assert!(record.is_clean());

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn audited_runner_logs_success_for_autotune_candidate() {
        let dir = temp_dir("audited-success");
        let audit_path = dir.join("audit.jsonl");
        let action = TestAction {
            id: "test-candidate",
            safety_class: SafetyClass::ReversibleLowRisk,
            should_fail_apply: false,
            should_fail_verify: false,
            affected_tasks: 31,
        };

        let result = run_audited_action_with_audit_path(
            "autotune candidate",
            &action,
            apply_policy(),
            &audit_path,
        )
        .unwrap();

        assert_eq!(result.state.affected_tasks, 31);
        assert!(result.rollback.is_some());

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(events.len(), 5);
        let terminal = events.last().expect("expected terminal audit event");
        assert_eq!(terminal.command, "autotune candidate");
        assert_eq!(terminal.action_id.as_deref(), Some("test-candidate"));
        assert_eq!(terminal.safety_class, Some(SafetyClass::ReversibleLowRisk));
        assert!(!terminal.dry_run);
        assert!(terminal.success);
        assert_eq!(terminal.affected_tasks, 31);
        assert!(terminal.message.contains("action applied and verified"));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn audited_runner_logs_apply_failure_for_autotune_candidate() {
        let dir = temp_dir("audited-apply-failure");
        let audit_path = dir.join("audit.jsonl");
        let action = TestAction {
            id: "test-candidate",
            safety_class: SafetyClass::ReversibleLowRisk,
            should_fail_apply: true,
            should_fail_verify: false,
            affected_tasks: 31,
        };

        let result = run_audited_action_with_audit_path(
            "autotune candidate",
            &action,
            apply_policy(),
            &audit_path,
        );

        assert!(result.is_err());

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(events.len(), 3);
        let terminal = events.last().expect("expected terminal audit event");
        assert_eq!(terminal.command, "autotune candidate");
        assert!(!terminal.success);
        assert!(terminal.message.contains("apply failed"));
        assert!(terminal.message.contains("intentional apply failure"));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn audited_runner_logs_verify_failure_for_autotune_candidate() {
        let dir = temp_dir("audited-verify-failure");
        let audit_path = dir.join("audit.jsonl");
        let action = TestAction {
            id: "test-candidate",
            safety_class: SafetyClass::ReversibleLowRisk,
            should_fail_apply: false,
            should_fail_verify: true,
            affected_tasks: 31,
        };

        let result = run_audited_action_with_audit_path(
            "autotune candidate",
            &action,
            apply_policy(),
            &audit_path,
        );

        assert!(result.is_err());

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(events.len(), 5);
        let terminal = events.last().expect("expected terminal audit event");
        assert_eq!(terminal.command, "autotune candidate");
        assert!(!terminal.success);
        assert!(terminal.message.contains("verify failed"));
        assert!(terminal.message.contains("intentional verify failure"));

        fs::remove_dir_all(dir).ok();
    }
}
