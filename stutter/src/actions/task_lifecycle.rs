//! Real-kernel acceptance of a per-task mutation on a caller-owned disposable tree: dry-run,
//! audited apply, durable identity-complete token, independent readback, rollback, independent
//! readback, and a second rollback that must change nothing. It does not bypass the policy, the
//! audited runner or the rollback. One implementation serves every per-task operation; an
//! operation supplies how its state is read, what it becomes, and how its token is shaped.
use std::{collections::BTreeMap, path::Path};

use anyhow::{Context, ensure};
use serde_json::{Value, json};

use super::{
    ActionPhase, RollbackToken, TaskIdentity, TuningAction,
    runner::{ActionRunPolicy, run_audited_action_with_audit_path},
};

/// Task id -> (task start time, observable state before the mutation).
pub(crate) type Originals = BTreeMap<u32, (u64, String)>;

pub(crate) struct LifecycleSpec<'a, A> {
    pub label: &'static str,
    pub tree_pid: u32,
    pub expected_starttime: u64,
    pub audit_path: &'a Path,
    /// The policy of the apply step (the explicit unlock a validation harness is granted).
    pub apply_policy: ActionRunPolicy,
    /// Builds the action for the tasks of the owned tree.
    pub build: &'a dyn Fn(Vec<TaskIdentity>) -> A,
    /// The observable state of one task, read straight from the kernel.
    pub read: &'a dyn Fn(u32) -> anyhow::Result<String>,
    /// The state a task has once the mutation is applied, derived from its original state.
    pub applied: &'a dyn Fn(&str) -> anyhow::Result<String>,
    /// Checks the durable token against what the harness read before the mutation.
    pub check_token: &'a dyn Fn(&RollbackToken, &Originals) -> anyhow::Result<()>,
}

type StateFn<'a> = dyn Fn(&str) -> anyhow::Result<String> + 'a;

fn readback(
    spec_read: &dyn Fn(u32) -> anyhow::Result<String>,
    original: &Originals,
    expected: Option<&StateFn<'_>>,
) -> anyhow::Result<()> {
    for (tid, (ticks, before)) in original {
        ensure!(
            crate::process_tree::process_starttime_at(Path::new("/proc"), *tid) == Some(*ticks),
            "acceptance task identity changed: {tid}"
        );
        let want = match expected {
            Some(applied) => applied(before)?,
            None => before.clone(),
        };
        let now = spec_read(*tid)?;
        ensure!(
            now == want,
            "kernel readback differs for task {tid}: {now} != {want}"
        );
    }
    Ok(())
}

pub(crate) fn verify_task_lifecycle<A: TuningAction>(
    spec: LifecycleSpec<'_, A>,
) -> anyhow::Result<Value> {
    let LifecycleSpec {
        label,
        tree_pid,
        expected_starttime,
        audit_path,
        apply_policy,
        build,
        read,
        applied,
        check_token,
    } = spec;
    ensure!(!audit_path.exists(), "acceptance audit path must be new");
    ensure!(
        crate::process_tree::process_starttime_at(Path::new("/proc"), tree_pid)
            == Some(expected_starttime),
        "acceptance root identity mismatch"
    );
    let snapshot = crate::process_tree::target_snapshot(
        crate::process_tree::TargetSnapshotInput::default().tree_pids(&[tree_pid]),
    );
    ensure!(
        (3..=8).contains(&snapshot.tasks.len()),
        "acceptance expects a stable disposable tree of 3 to 8 tasks"
    );
    let process_count = snapshot
        .tasks
        .values()
        .map(|task| task.process_id())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    ensure!(
        process_count == 3,
        "acceptance requires parent plus two disposable child processes"
    );
    let mut original = Originals::new();
    for task in snapshot.tasks.values() {
        let tid = task.task_id().as_u32();
        let state = read(tid)?;
        ensure!(
            applied(&state)? != state,
            "no observable {label} change for task {tid}"
        );
        let ticks = crate::process_tree::process_starttime_at(Path::new("/proc"), tid)
            .context("task identity missing")?;
        original.insert(tid, (ticks, state));
    }
    let action = build(
        snapshot
            .tasks
            .values()
            .map(TaskIdentity::from_task_info)
            .collect(),
    );
    let source = crate::daemon_policy::ActionSource::Cli;
    let mut dry_policy = ActionRunPolicy::dry_run(source);
    dry_policy.max_affected_tasks = Some(8);
    let dry = run_audited_action_with_audit_path(
        &format!("{label}-acceptance-dry-run"),
        &action,
        dry_policy,
        audit_path,
    )?;
    ensure!(
        dry.state.affected_tasks == original.len() && !dry.state.applied,
        "dry-run scope mismatch"
    );
    readback(&read, &original, None)?;
    let mut policy = apply_policy;
    policy.max_affected_tasks = Some(8);
    policy.max_total_duration = Some(std::time::Duration::from_secs(10));
    let result = run_audited_action_with_audit_path(
        &format!("{label}-acceptance-apply"),
        &action,
        policy,
        audit_path,
    )?;
    let token = result
        .rollback
        .clone()
        .context("applied action omitted its rollback token")?;
    // Every assertion after the mutation stays in this closure so rollback always runs.
    let verified = (|| -> anyhow::Result<()> {
        check_token(&token, &original)?;
        ensure!(
            result.state.applied && result.state.checked_tasks == original.len(),
            "action did not verify complete scope"
        );
        readback(&read, &original, Some(applied))
    })();
    let rollback = action.rollback(&token);
    let restored = readback(&read, &original, None);
    let mut events = crate::audit::read_audit_tail(audit_path, 100)?;
    let mut event = events.last().cloned().context("action audit missing")?;
    event.unix_nanos = crate::audit::unix_nanos_now();
    event.command = format!("{label}-acceptance-rollback");
    event.action_phase = Some(ActionPhase::Rollback);
    event.success = rollback.is_ok() && restored.is_ok();
    event.message = format!("rollback={rollback:?}; independent_kernel_restoration={restored:?}");
    crate::audit::append_audit_event_to_path(audit_path, &event)?;
    rollback?;
    restored?;
    verified?;
    events = crate::audit::read_audit_tail(audit_path, 100)?;
    for phase in [
        ActionPhase::Preflight,
        ActionPhase::DryRun,
        ActionPhase::Apply,
        ActionPhase::Verify,
        ActionPhase::Rollback,
    ] {
        ensure!(
            events
                .iter()
                .any(|event| event.action_phase == Some(phase) && event.success),
            "missing successful {phase:?} audit event"
        );
    }
    // Rolling back again must be a no-op that changes nothing (idempotent restore).
    action.rollback(&token)?;
    readback(&read, &original, None)?;
    Ok(json!({
        "target_pid": tree_pid,
        "target_starttime_ticks": expected_starttime,
        "tasks": original.iter().map(|(tid, (ticks, before))| json!({
            "tid": tid, "starttime_ticks": ticks, "original": before,
            "applied": applied(before).ok(),
        })).collect::<Vec<_>>(),
        "process_count": process_count,
        "audit_events": events.len(),
        "dry_run_unchanged": true,
        "durable_rollback_verified": true,
        "independent_kernel_restore_verified": true,
        "rollback_idempotent": true,
    }))
}
