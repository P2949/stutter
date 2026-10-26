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
        nice::{NiceAction, NicePolicy},
    },
    autotune::{
        objective::ObjectiveKind,
        planning::{
            candidate::{CandidateAction, CandidateEvidence},
            executable_plan::NiceActionPlan,
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
    matches!(token, RollbackToken::NiceRestore { .. })
}

/// Restores `token` through the worker listening on `socket`; returns the restored task count.
pub(super) fn restore_through_worker(
    socket: &std::path::Path,
    token: &RollbackToken,
) -> anyhow::Result<usize> {
    let Some(records) = token.as_nice_restore() else {
        anyhow::bail!("only nice tokens are restored through the privileged worker");
    };
    let targets: Vec<TaskIdentity> = records
        .iter()
        .map(|record| {
            let identity = record.restore_identity();
            TaskIdentity {
                tid: identity.tid,
                process_pid: identity.process_pid,
                comm: identity.comm,
                starttime_ticks: identity.starttime_ticks,
            }
        })
        .collect();
    let candidate = CandidateAction::Nice {
        plan: NiceActionPlan {
            name: "startup-recovery-nice".to_owned(),
            action: NiceAction {
                targets,
                nice: 0,
                policy: NicePolicy::default(),
            },
            target_root_pid: None,
            evidence: vec![CandidateEvidence::new(
                "startup_recovery",
                "interrupted nice demotion",
                1.0,
            )],
            objective: ObjectiveKind::GameRunnableLatency,
        },
    };
    let result = UnixSocketPrivilegedActionService::new(socket)
        .rollback(RollbackRequest {
            candidate,
            token: token.clone(),
            policy: DaemonPolicy::apply_medium_risk(ActionSource::AutotuneRuntime),
            context: DaemonPolicyContext::default(),
        })
        .context("the privileged worker could not restore the interrupted nice demotion")?;
    Ok(result.affected_tasks)
}
