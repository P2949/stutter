//! Root-owned record of issued service tokens. A private socket authenticates the controller;
//! only this durable ledger authorizes privileged restoration of caller-supplied token data.
use serde::{Deserialize, Serialize};

use super::*;

#[derive(Debug)]
pub(super) struct ServiceActionService {
    inner: InProcessPrivilegedActionService,
    ledger: PathBuf,
    targets: super::service_targets::ServiceTargets,
}

#[derive(Serialize, Deserialize)]
struct IssuedToken {
    boot_id: String,
    token: RollbackToken,
    plan: PrivilegedWorkerCandidatePlan,
    #[serde(default)]
    restored: bool,
}

impl ServiceActionService {
    pub(super) fn new(home: &Path, uid: u32, cgroup: Option<PathBuf>) -> Self {
        Self {
            inner: InProcessPrivilegedActionService::default(),
            ledger: home.join("issued-tokens.json"),
            targets: super::service_targets::ServiceTargets::new(uid, cgroup),
        }
    }

    /// Worker restart settles every root-issued token before accepting a new connection. The
    /// controller can subsequently repeat its own recovery; completed tokens are idempotent.
    pub(super) fn recover_issued(
        &self,
        policy: &crate::daemon_policy::DaemonPolicy,
    ) -> anyhow::Result<()> {
        for issued in self.read()?.into_iter().filter(|issued| !issued.restored) {
            self.rollback(RollbackRequest {
                candidate: issued.plan.into_plan_request()?.candidate,
                token: issued.token,
                policy: policy.clone(),
                context: Default::default(),
            })?;
        }
        Ok(())
    }

    /// A transaction that stopped before its first mutable write leaves nothing to restore,
    /// whichever boundary caught it: the token is void, and the controller learns by type that the
    /// candidate is stale. Any other outcome passes through untouched.
    fn settle_precommit_abort(
        &self,
        outcome: anyhow::Result<ApplyResult>,
        recorded: Option<&RollbackToken>,
    ) -> anyhow::Result<ApplyResult> {
        let Err(error) = &outcome else {
            return outcome;
        };
        let Some(abort) = crate::actions::precommit::AbortedBeforeMutation::in_chain(error) else {
            return outcome;
        };
        if let Some(token) = recorded {
            self.void_unapplied(token, error)?;
        }
        Err(PrivilegedWorkerError::StaleTargetBeforeMutation {
            detail: format!("{:?}: {abort}", abort.reason),
        }
        .into())
    }

    /// The prepared transaction was abandoned before its first mutable write. The token is retired
    /// as void so no later recovery has anything to restore for it.
    fn void_unapplied(&self, token: &RollbackToken, why: &anyhow::Error) -> anyhow::Result<()> {
        let digest = rollback_snapshot_digest(token);
        let mut tokens = self.read()?;
        if let Some(issued) = tokens
            .iter_mut()
            .find(|issued| rollback_snapshot_digest(&issued.token) == digest && !issued.restored)
        {
            issued.restored = true;
            crate::audit::append_audit_event_to_path(
                &self.ledger.with_file_name("token-recovery-audit.jsonl"),
                &privileged_operation_audit_event(
                    PrivilegedOperation::RollbackAction,
                    true,
                    format!(
                        "worker-issued token voided digest={digest}: the prepared transaction was abandoned before its first mutable write ({why:#})"
                    ),
                ),
            )?;
            self.write(&tokens)?;
        }
        Ok(())
    }

    fn read(&self) -> anyhow::Result<Vec<IssuedToken>> {
        match fs::read(&self.ledger) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("read worker-issued token ledger"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(error.into()),
        }
    }

    fn write(&self, tokens: &[IssuedToken]) -> anyhow::Result<()> {
        let parent = self
            .ledger
            .parent()
            .context("worker token ledger needs parent")?;
        fs::create_dir_all(parent)?;
        let tmp = self.ledger.with_extension(format!(
            "tmp-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        serde_json::to_writer(&mut file, tokens)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, &self.ledger)?;
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    }

    fn record(&self, token: &RollbackToken, plan: &CandidatePlanRequest) -> anyhow::Result<()> {
        let mut tokens = self.read()?;
        let digest = rollback_snapshot_digest(token);
        if let Some(issued) = tokens
            .iter_mut()
            .find(|issued| rollback_snapshot_digest(&issued.token) == digest)
        {
            anyhow::ensure!(
                issued.restored,
                PrivilegedWorkerError::ServiceTokenAlreadyOutstanding
            );
            // Identical inline restoration data may recur after a completed transaction. It
            // becomes outstanding again before a new commit; an old tombstone cannot skip it.
            issued.restored = false;
            issued.boot_id = stutter_core::boot::current_boot_id()?;
            issued.plan = PrivilegedWorkerCandidatePlan::from_plan_request(plan);
        } else {
            tokens.push(IssuedToken {
                boot_id: stutter_core::boot::current_boot_id()?,
                token: token.clone(),
                plan: PrivilegedWorkerCandidatePlan::from_plan_request(plan),
                restored: false,
            });
        }
        self.write(&tokens)?;
        Ok(())
    }
}

impl PrivilegedActionService for ServiceActionService {
    fn settle_issued_tokens(
        &self,
        policy: &crate::daemon_policy::DaemonPolicy,
    ) -> anyhow::Result<usize> {
        let outstanding = self
            .read()?
            .iter()
            .filter(|issued| !issued.restored)
            .count();
        self.recover_issued(policy)?;
        Ok(outstanding)
    }
    fn dry_run_candidate(
        &self,
        request: CandidateApplyRequest,
    ) -> anyhow::Result<crate::autotune::planning::dry_run::CandidateDryRunRecord> {
        super::service_targets::ServiceTargets::policy(&request.plan.candidate, &request.policy)?;
        self.targets.candidate(&request.plan.candidate)?;
        self.inner.dry_run_candidate(request)
    }
    fn apply_candidate(&self, _: CandidateApplyRequest) -> anyhow::Result<ApplyResult> {
        Err(PrivilegedWorkerError::BoundedServiceRequiresPrepared.into())
    }
    fn apply_candidate_prepared(
        &self,
        request: CandidateApplyRequest,
        prepared: &mut PreparedHook<'_>,
    ) -> anyhow::Result<ApplyResult> {
        let plan = request.plan.clone();
        super::service_targets::ServiceTargets::policy(&plan.candidate, &request.policy)?;
        self.targets.candidate(&plan.candidate)?;
        let recorded = std::cell::RefCell::new(None::<RollbackToken>);
        let outcome = self.inner.apply_candidate_prepared(request, &mut |token| {
            // Root's token and boot identity are durable before the controller gets a prepare
            // answer and before the first mutable write. A crash retains the authority to restore.
            self.targets.prepared(&plan.candidate, token)?;
            self.record(token, &plan)?;
            *recorded.borrow_mut() = Some(token.clone());
            let barrier =
                prepared(token).and_then(|()| self.targets.prepared(&plan.candidate, token));
            if let Err(error) = &barrier {
                // Nothing was written for this token and nothing will be: it is void, not
                // outstanding, and the audit says so truthfully.
                self.void_unapplied(token, error)?;
            }
            barrier
        });
        self.settle_precommit_abort(outcome, recorded.borrow().as_ref())
    }
    fn rollback(&self, mut request: RollbackRequest) -> anyhow::Result<RollbackResult> {
        let mut tokens = self.read()?;
        let digest = rollback_snapshot_digest(&request.token);
        let Some(index) = tokens
            .iter()
            .position(|issued| rollback_snapshot_digest(&issued.token) == digest)
        else {
            return Err(PrivilegedWorkerError::ServiceRollbackTokenNotIssued.into());
        };
        let issued = &tokens[index];
        anyhow::ensure!(
            stutter_core::boot::valid_boot_id(&issued.boot_id),
            "malformed worker-issued boot identity"
        );
        if issued.restored {
            return Ok(RollbackResult { affected_tasks: 0 });
        }
        // Resource roots and executors come from the root-issued plan, never the caller.
        request.candidate = issued.plan.clone().into_plan_request()?.candidate;
        // A root-owned snapshot is removed only after a completed physical restore. If the
        // worker died between that unlink and its ledger update, repeating recovery is safe.
        crate::actions::disposition::RollbackDisposition::reset();
        let already_restored = matches!(&issued.token, RollbackToken::CpuAffinityRestoreFile { path, .. } if !path.exists());
        let boot_superseded = issued.boot_id != stutter_core::boot::current_boot_id()?;
        let token_kind = issued.token.clone();
        // Autonomous recovery never infers ownership from missing metadata.
        if !already_restored
            && !boot_superseded
            && let RollbackToken::CgroupRestore { records, .. } = &token_kind
            && records
                .iter()
                .any(|record| record.applied_cgroup.is_none() || record.applied_cgroup_id.is_none())
        {
            crate::audit::append_audit_event_to_path(
                &self.ledger.with_file_name("token-recovery-audit.jsonl"),
                &privileged_operation_audit_event(
                    PrivilegedOperation::RollbackAction,
                    false,
                    format!(
                        "worker-issued token refused digest={digest}: a cgroup record lacks the applied placement identity disposition='{}'",
                        crate::actions::disposition::RollbackDisposition::default()
                    ),
                ),
            )?;
            return Err(PrivilegedWorkerError::ServiceRollbackUnresolved.into());
        }
        let outcome: anyhow::Result<RollbackResult> = if already_restored {
            Ok(RollbackResult { affected_tasks: 0 })
        } else if boot_superseded {
            // Task identities cannot survive reboot; resource tokens enforce their own policy.
            match &token_kind {
                RollbackToken::CpuAffinityRestoreFile { .. }
                | RollbackToken::CgroupRestore { .. }
                | RollbackToken::NiceRestore { .. }
                | RollbackToken::UclampRestore { .. }
                | RollbackToken::IoPrioRestore { .. } => Ok(RollbackResult { affected_tasks: 0 }),
                RollbackToken::CpuPowerRestore { .. }
                | RollbackToken::IrqAffinityRestore { .. }
                | RollbackToken::GpuPowerRestore { .. }
                | RollbackToken::VmKnobRestore { .. }
                | RollbackToken::SysfsRestore { .. } => self.inner.rollback(request),
            }
        } else {
            self.inner.rollback(request)
        };
        // The disposition is collected whether the rollback succeeded or not: the root ledger is
        // the authority on what physically happened, and a refusal is evidence too.
        let disposition = crate::actions::disposition::RollbackDisposition::take();
        let audit = |success: bool, outcome: &str| -> anyhow::Result<()> {
            crate::audit::append_audit_event_to_path(
                &self.ledger.with_file_name("token-recovery-audit.jsonl"),
                &privileged_operation_audit_event(
                    PrivilegedOperation::RollbackAction,
                    success,
                    format!(
                        "worker-issued token {outcome} digest={digest} boot_superseded={boot_superseded} already_restored={already_restored} disposition='{disposition}'"
                    ),
                ),
            )
        };
        match outcome {
            Ok(result) if disposition.unresolved == 0 => {
                audit(true, "settled")?;
                tokens[index].restored = true;
                self.write(&tokens)?;
                Ok(result)
            }
            Ok(_) => {
                audit(false, "left unresolved")?;
                Err(PrivilegedWorkerError::ServiceRollbackUnresolved.into())
            }
            Err(error) => {
                audit(false, &format!("failed ({error:#})"))?;
                if disposition.unresolved > 0 {
                    // The token stays outstanding: a live task is still in Stutter's state.
                    Err(
                        anyhow::Error::new(PrivilegedWorkerError::ServiceRollbackUnresolved)
                            .context(format!("{error:#}")),
                    )
                } else {
                    Err(error)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(token: RollbackToken) -> RollbackRequest {
        RollbackRequest {
            candidate:
                crate::autotune::planning::candidate::CandidateAction::cpu_affinity_profile_for(
                    crate::profiles::Profile {
                        name: "recovery".into(),
                        rules: vec![],
                    },
                    std::process::id(),
                    crate::autotune::objective::ObjectiveKind::GameRunnableLatency,
                ),
            token,
            policy: crate::daemon_policy::DaemonPolicy::apply_medium_risk(
                crate::daemon_policy::ActionSource::Test,
            ),
            context: Default::default(),
        }
    }
    #[test]
    fn settling_takes_no_input_and_counts_only_its_own_outstanding_tokens() {
        let root = crate::test_support::TestRoot::new("settle-service-token");
        let service = ServiceActionService::new(&root.join("state"), 1000, None);
        let policy = crate::daemon_policy::DaemonPolicy::apply_medium_risk(
            crate::daemon_policy::ActionSource::Test,
        );
        assert_eq!(service.settle_issued_tokens(&policy).unwrap(), 0);
        let token = RollbackToken::CpuAffinityRestoreFile {
            path: root.join("restored-and-unlinked.json"),
            affected_tasks: 1,
        };
        let candidate = request(token.clone()).candidate;
        service
            .record(
                &token,
                &CandidatePlanRequest::from_candidate(candidate, crate::audit::unix_nanos_now()),
            )
            .unwrap();
        assert_eq!(service.settle_issued_tokens(&policy).unwrap(), 1);
        assert_eq!(service.settle_issued_tokens(&policy).unwrap(), 0);
    }

    #[test]
    fn fabricated_token_is_refused_and_issued_restore_is_idempotent() {
        let root = crate::test_support::TestRoot::new("issued-service-token");
        let service = ServiceActionService::new(&root.join("state"), 1000, None);
        let token = RollbackToken::CpuAffinityRestoreFile {
            path: root.join("restored-and-unlinked.json"),
            affected_tasks: 1,
        };
        let error = service.rollback(request(token.clone())).unwrap_err();
        assert!(format!("{error:#}").contains("service_rollback_token_not_issued"));
        let candidate = request(token.clone()).candidate;
        service
            .record(
                &token,
                &CandidatePlanRequest::from_candidate(candidate, crate::audit::unix_nanos_now()),
            )
            .unwrap();
        assert_eq!(
            service
                .rollback(request(token.clone()))
                .unwrap()
                .affected_tasks,
            0
        );
        assert_eq!(service.rollback(request(token)).unwrap().affected_tasks, 0);
        let forged = RollbackToken::CpuAffinityRestoreFile {
            path: root.join("forged.json"),
            affected_tasks: 1,
        };
        assert!(service.rollback(request(forged)).is_err());
    }

    #[test]
    fn another_accounts_target_is_rejected_before_snapshot_or_commit() {
        use std::os::unix::fs::MetadataExt;
        let root = crate::test_support::TestRoot::new("service-target-denied-before-prepare");
        let actual_uid = fs::metadata("/proc/self").unwrap().uid();
        let service = ServiceActionService::new(root.path(), actual_uid + 1, None);
        let candidate = request(RollbackToken::CpuAffinityRestoreFile {
            path: root.join("never-created.json"),
            affected_tasks: 0,
        })
        .candidate;
        let error = service
            .apply_candidate_prepared(
                CandidateApplyRequest {
                    plan: CandidatePlanRequest::from_candidate(
                        candidate,
                        crate::audit::unix_nanos_now(),
                    ),
                    policy: crate::daemon_policy::DaemonPolicy::apply_medium_risk(
                        crate::daemon_policy::ActionSource::Test,
                    ),
                    context: Default::default(),
                    max_plan_age_nanos: 1_000_000_000,
                },
                &mut |_| panic!("a denied target must never reach the commit barrier"),
            )
            .unwrap_err();
        assert!(format!("{error:#}").contains("bounded_service_target_uid_denied"));
        assert!(!service.ledger.exists());
        assert!(!root.join("never-created.json").exists());
    }
}

#[cfg(test)]
mod void_tests {
    use super::*;

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

#[cfg(test)]
mod disposition_audit_tests {
    use super::*;

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

#[cfg(test)]
mod unresolved_audit_tests {
    use super::*;

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

#[cfg(test)]
mod precommit_abort_tests {
    use super::*;
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
