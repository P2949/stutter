//! Ledger audit behaviour: voided, settled and unresolved tokens, and typed pre-write aborts.
mod void_tests {
    use super::super::*;

    #[test]
    fn a_token_abandoned_before_its_first_write_is_void_and_never_outstanding() {
        let root = crate::test_support::TestRoot::new("void-service-token");
        let service = ServiceActionService::new(&root.join("state"), 1000, None);
        let policy = crate::daemon_policy::DaemonPolicy::apply_medium_risk(
            crate::daemon_policy::ActionSource::Test,
        );
        let token = RollbackToken::CpuAffinityRestoreFile {
            path: root.join("never-written.json"),
            affected_tasks: 1,
        };
        let plan = CandidatePlanRequest::from_candidate(
            crate::autotune::planning::candidate::CandidateAction::cpu_affinity_profile_for(
                crate::profiles::Profile {
                    name: "void".into(),
                    rules: vec![],
                },
                std::process::id(),
                crate::autotune::objective::ObjectiveKind::GameRunnableLatency,
            ),
            crate::audit::unix_nanos_now(),
        );
        service.record(&token, &plan).unwrap();
        assert_eq!(service.settle_issued_tokens(&policy).unwrap(), 1);
        // A second prepare of the same snapshot after a void is allowed again.
        service.record(&token, &plan).unwrap();
        service
            .void_unapplied(&token, &anyhow::anyhow!("task exited"))
            .unwrap();
        assert_eq!(service.settle_issued_tokens(&policy).unwrap(), 0);
        let audit = fs::read_to_string(root.join("state/token-recovery-audit.jsonl")).unwrap();
        assert!(audit.contains("voided"));
    }
}

mod disposition_audit_tests {
    use super::super::*;

    fn starttime(path: &str) -> u64 {
        let stat = fs::read_to_string(path).unwrap();
        stat.rsplit_once(") ")
            .unwrap()
            .1
            .split_whitespace()
            .nth(19)
            .unwrap()
            .parse()
            .unwrap()
    }

    #[test]
    fn an_external_takeover_settles_the_token_and_the_audit_says_so() {
        let root = crate::test_support::TestRoot::new("external-takeover-audit");
        let service = ServiceActionService::new(&root.join("state"), 1000, None);
        let policy = crate::daemon_policy::DaemonPolicy::apply_medium_risk(
            crate::daemon_policy::ActionSource::Test,
        );
        let pid = std::process::id();
        let ticks = starttime("/proc/self/stat");
        let live = crate::affinity::read_allowed_mask_raw(pid).unwrap();
        // Stutter claims to have applied a mask that is not the one the task has now.
        let applied = crate::affinity::CpuMask::parse("1022").unwrap();
        assert_ne!(live, applied);
        let path = root.join("restore.json");
        crate::profile_restore::save_restore_state(
            &path,
            &crate::profile_restore::ProfileRestoreState {
                schema_version: crate::profile_restore::PROFILE_RESTORE_SCHEMA_VERSION,
                affinity_records: vec![crate::affinity::AffinityRecord {
                    tid: pid.into(),
                    process_pid: Some(pid.into()),
                    process_starttime_ticks: Some(ticks),
                    task_starttime_ticks: Some(starttime(&format!("/proc/self/task/{pid}/stat"))),
                    original_mask: live.clone(),
                    applied_mask: applied,
                }],
                ..Default::default()
            },
        )
        .unwrap();
        let token = RollbackToken::CpuAffinityRestoreFile {
            path: path.clone(),
            affected_tasks: 1,
        };
        let candidate =
            crate::autotune::planning::candidate::CandidateAction::cpu_affinity_profile_for(
                crate::profiles::Profile {
                    name: "takeover".into(),
                    rules: vec![],
                },
                pid,
                crate::autotune::objective::ObjectiveKind::GameRunnableLatency,
            );
        service
            .record(
                &token,
                &CandidatePlanRequest::from_candidate(candidate, crate::audit::unix_nanos_now()),
            )
            .unwrap();
        service.settle_issued_tokens(&policy).unwrap();
        let audit = fs::read_to_string(root.join("state/token-recovery-audit.jsonl")).unwrap();
        assert!(audit.contains("superseded_externally=1"), "{audit}");
        assert!(audit.contains("unresolved=0"));
        assert!(audit.contains("settled"));
        // The external value was not overwritten.
        assert_eq!(crate::affinity::read_allowed_mask_raw(pid).unwrap(), live);
    }
}

mod unresolved_audit_tests {
    use super::super::*;

    #[test]
    fn a_live_task_whose_origin_cannot_be_resolved_leaves_the_token_open_and_the_audit_says_so() {
        let root = crate::test_support::TestRoot::new("unresolved-audit");
        let service = ServiceActionService::new(&root.join("state"), 1000, None);
        let policy = crate::daemon_policy::DaemonPolicy::apply_medium_risk(
            crate::daemon_policy::ActionSource::Test,
        );
        let pid = std::process::id();
        let membership = fs::read_to_string("/proc/self/cgroup").unwrap();
        let live = std::path::PathBuf::from(
            membership
                .lines()
                .find_map(|line| line.strip_prefix("0::"))
                .unwrap()
                .trim(),
        );
        let applied_id = std::fs::metadata(
            std::path::Path::new("/sys/fs/cgroup").join(live.strip_prefix("/").unwrap()),
        )
        .map(|metadata| std::os::unix::fs::MetadataExt::ino(&metadata))
        .unwrap_or(0);
        let ticks: u64 = fs::read_to_string("/proc/self/stat")
            .unwrap()
            .rsplit_once(") ")
            .unwrap()
            .1
            .split_whitespace()
            .nth(19)
            .unwrap()
            .parse()
            .unwrap();
        let identity = crate::actions::TaskRestoreIdentity::observed(
            pid,
            Some(pid),
            Some("test".to_owned()),
            Some(ticks),
            None,
        );
        let token = RollbackToken::CgroupRestore {
            records: vec![
                crate::actions::CgroupRestoreRecord::new(
                    identity,
                    std::path::PathBuf::from("/this/origin/does/not/exist"),
                )
                .with_applied(live, applied_id),
            ],
            cpuset: None,
        };
        let candidate = crate::autotune::planning::candidate::CandidateAction::CgroupPlacement {
            plan: crate::autotune::planning::executable_plan::CgroupPlacementActionPlan {
                target_root_starttime_ticks: None,
                name: "unresolved".to_owned(),
                action: crate::actions::cgroup::CgroupPlacementAction {
                    cgroup_root: "/sys/fs/cgroup".into(),
                    target_cgroup: "/sys/fs/cgroup/game.slice".into(),
                    targets: vec![crate::actions::cgroup::CgroupPlacementTarget {
                        identity: crate::actions::TaskIdentity {
                            tid: pid.into(),
                            process_pid: Some(pid.into()),
                            comm: None,
                            starttime_ticks: Some(ticks),
                        },
                        class: crate::process_tree::TaskClass::Game,
                    }],
                    cpuset_cpus: None,
                    cpuset_mems: None,
                },
                target_root_pid: Some(pid),
                evidence: Vec::new(),
                objective: crate::autotune::objective::ObjectiveKind::GameRunnableLatency,
            },
        };
        service
            .record(
                &token,
                &CandidatePlanRequest::from_candidate(candidate, crate::audit::unix_nanos_now()),
            )
            .unwrap();
        let error = service.settle_issued_tokens(&policy).unwrap_err();
        assert!(
            format!("{error:#}").contains("service_rollback_unresolved")
                || format!("{error:#}").contains("unresolved"),
            "{error:#}"
        );
        let audit = fs::read_to_string(root.join("state/token-recovery-audit.jsonl")).unwrap();
        assert!(audit.contains("unresolved=1"), "{audit}");
        assert!(audit.contains("\"success\":false"));
        // The token is still open: nothing was retired.
        let report = fs::read_to_string(root.join("state/issued-tokens.json")).unwrap();
        assert!(report.contains("\"restored\":false"), "{report}");
    }
}

mod precommit_abort_tests {
    use super::super::*;
    use crate::actions::precommit::{AbortReason, AbortedBeforeMutation};

    fn recorded_service(
        name: &str,
    ) -> (
        ServiceActionService,
        RollbackToken,
        crate::test_support::TestRoot,
    ) {
        let root = crate::test_support::TestRoot::new(name);
        let service = ServiceActionService::new(&root.join("state"), 1000, None);
        let token = RollbackToken::CpuAffinityRestoreFile {
            path: root.join("never-written.json"),
            affected_tasks: 1,
        };
        let plan = CandidatePlanRequest::from_candidate(
            crate::autotune::planning::candidate::CandidateAction::cpu_affinity_profile_for(
                crate::profiles::Profile {
                    name: "late".into(),
                    rules: vec![],
                },
                std::process::id(),
                crate::autotune::objective::ObjectiveKind::GameRunnableLatency,
            ),
            crate::audit::unix_nanos_now(),
        );
        service.record(&token, &plan).unwrap();
        (service, token, root)
    }

    #[test]
    fn an_abort_after_the_durable_record_voids_the_token_and_reports_a_stale_candidate() {
        let policy = crate::daemon_policy::DaemonPolicy::apply_medium_risk(
            crate::daemon_policy::ActionSource::Test,
        );
        for reason in [
            AbortReason::TargetGone,
            AbortReason::TargetReplaced,
            AbortReason::PlanDrifted,
        ] {
            let (service, token, root) = recorded_service("late-abort");
            // The root token is outstanding: the executor stopped after the record.
            let outcome = service.settle_precommit_abort(
                Err(
                    anyhow::Error::new(AbortedBeforeMutation::new(reason, "raced"))
                        .context("executor stopped before its first write"),
                ),
                Some(&token),
            );
            let error = outcome.unwrap_err();
            assert!(matches!(
                error.downcast_ref::<PrivilegedWorkerError>(),
                Some(PrivilegedWorkerError::StaleTargetBeforeMutation { .. })
            ));
            assert_eq!(service.settle_issued_tokens(&policy).unwrap(), 0);
            let audit = fs::read_to_string(root.join("state/token-recovery-audit.jsonl")).unwrap();
            assert!(audit.contains("voided"), "{audit}");
        }
    }

    #[test]
    fn any_other_failure_keeps_the_token_outstanding_for_recovery() {
        let policy = crate::daemon_policy::DaemonPolicy::apply_medium_risk(
            crate::daemon_policy::ActionSource::Test,
        );
        let (service, token, _root) = recorded_service("late-other");
        let outcome =
            service.settle_precommit_abort(Err(anyhow::anyhow!("unreadable")), Some(&token));
        assert!(
            outcome
                .unwrap_err()
                .downcast_ref::<PrivilegedWorkerError>()
                .is_none()
        );
        assert_eq!(service.settle_issued_tokens(&policy).unwrap(), 1);
    }
}
