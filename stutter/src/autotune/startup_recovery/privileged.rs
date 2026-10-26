//! Startup recovery of tokens whose restore needs privilege.
//!
//! Lowering a task's niceness again is refused to an ordinary user (`RLIMIT_NICE` is 0), so a
//! crashed controller's nice demotion can only be undone by the privileged worker, through the same
//! `Rollback` request, policy and identity checks as a normal revert. The worker derives its
//! executor from a candidate plan, so a plan is rebuilt from the token's own identity records.
use anyhow::Context;

use crate::{
    actions::{
        RollbackToken, TaskIdentity,
        cgroup::{CgroupPlacementAction, CgroupPlacementTarget},
        nice::{NiceAction, NicePolicy},
    },
    autotune::{
        objective::ObjectiveKind,
        planning::{
            candidate::{CandidateAction, CandidateEvidence},
            executable_plan::{CgroupPlacementActionPlan, NiceActionPlan},
        },
    },
    daemon::{
        DaemonPolicy,
        policy::DaemonPolicyContext,
        privilege::{PrivilegedActionService, RollbackRequest, UnixSocketPrivilegedActionService},
    },
    daemon_policy::ActionSource,
};

/// Whether restoring `token` needs the privileged worker.
pub(super) fn needs_privileged_restore(token: &RollbackToken) -> bool {
    matches!(
        token,
        RollbackToken::NiceRestore { .. } | RollbackToken::CgroupRestore { .. }
    )
}

/// The candidate the worker derives its rollback executor from, rebuilt from the token's own
/// identity records. Refuses a token with any record whose identity is not complete: a legacy
/// record is never restored automatically.
fn candidate_for(token: &RollbackToken) -> anyhow::Result<CandidateAction> {
    let evidence = vec![CandidateEvidence::new(
        "startup_recovery",
        "interrupted privileged action",
        1.0,
    )];
    let complete = |identity: &crate::actions::TaskRestoreIdentity| {
        identity.process_pid.is_some() && identity.starttime_ticks.is_some()
    };
    let to_target = |identity: crate::actions::TaskRestoreIdentity| TaskIdentity {
        tid: identity.tid,
        process_pid: identity.process_pid,
        comm: identity.comm,
        starttime_ticks: identity.starttime_ticks,
    };
    match token {
        RollbackToken::NiceRestore { records } => {
            anyhow::ensure!(
                records.iter().all(|r| complete(&r.restore_identity())),
                "a nice record without a complete task identity is never restored automatically"
            );
            Ok(CandidateAction::Nice {
                plan: NiceActionPlan {
                    name: "startup-recovery-nice".to_owned(),
                    action: NiceAction {
                        targets: records
                            .iter()
                            .map(|r| to_target(r.restore_identity()))
                            .collect(),
                        nice: 0,
                        policy: NicePolicy::default(),
                    },
                    target_root_pid: None,
                    evidence,
                    objective: ObjectiveKind::GameRunnableLatency,
                },
            })
        }
        RollbackToken::CgroupRestore { records, .. } => {
            anyhow::ensure!(
                records
                    .iter()
                    .all(|r| complete(&r.restore_identity()) && r.original_cgroup_id.is_some()),
                "a cgroup record without a complete task and cgroup identity is never restored automatically"
            );
            let original = records
                .first()
                .map(|r| r.original_cgroup.clone())
                .unwrap_or_default();
            Ok(CandidateAction::CgroupPlacement {
                plan: CgroupPlacementActionPlan {
                    name: "startup-recovery-cgroup".to_owned(),
                    action: CgroupPlacementAction {
                        cgroup_root: std::path::PathBuf::from("/sys/fs/cgroup"),
                        target_cgroup: original,
                        targets: records
                            .iter()
                            .map(|r| CgroupPlacementTarget {
                                identity: to_target(r.restore_identity()),
                                class: crate::process_tree::TaskClass::Game,
                            })
                            .collect(),
                        cpuset_cpus: None,
                        cpuset_mems: None,
                    },
                    target_root_pid: None,
                    evidence,
                    objective: ObjectiveKind::GameRunnableLatency,
                },
            })
        }
        _ => anyhow::bail!("this token is not restored through the privileged worker"),
    }
}

/// Restores `token` through the worker listening on `socket`; returns the restored task count.
pub(super) fn restore_through_worker(
    socket: &std::path::Path,
    token: &RollbackToken,
) -> anyhow::Result<usize> {
    let result = UnixSocketPrivilegedActionService::new(socket)
        .rollback(RollbackRequest {
            candidate: candidate_for(token)?,
            token: token.clone(),
            policy: DaemonPolicy::apply_medium_risk(ActionSource::AutotuneRuntime),
            context: DaemonPolicyContext::default(),
        })
        .context("the privileged worker could not restore the interrupted action")?;
    Ok(result.affected_tasks)
}
