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

/// Settled tokens the ledger keeps as tombstones, so that a repeated restore of a recent token
/// stays an idempotent no-op. Older settled entries are dropped when a token is recorded, which
/// keeps every rewrite of the ledger bounded; the recovery audit keeps their history. A token that
/// is still outstanding is never dropped.
const SETTLED_RETAINED: usize = 512;

/// Drops the oldest settled tokens beyond [`SETTLED_RETAINED`], leaving every outstanding token
/// and the order of the rest untouched.
fn compact_settled(tokens: &mut Vec<IssuedToken>) {
    let mut surplus = tokens
        .iter()
        .filter(|issued| issued.restored)
        .count()
        .saturating_sub(SETTLED_RETAINED);
    if surplus == 0 {
        return;
    }
    tokens.retain(|issued| {
        if issued.restored && surplus > 0 {
            surplus -= 1;
            return false;
        }
        true
    });
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
        compact_settled(&mut tokens);
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
                | RollbackToken::IoPrioRestore { .. }
                // A PM QoS lease cannot outlive the boot that held it.
                | RollbackToken::PmQosRelease { .. } => Ok(RollbackResult { affected_tasks: 0 }),
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
mod audit_tests;
