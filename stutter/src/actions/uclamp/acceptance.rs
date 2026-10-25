//! Real-kernel acceptance of the uclamp action on a caller-owned disposable tree: dry-run, audited
//! apply, durable identity-complete token, independent readback, rollback, independent readback.
//! It does not bypass the policy, the audited runner or the rollback.
use std::{collections::BTreeMap, path::Path};

use anyhow::{Context, ensure};
use serde_json::{Value, json};

use super::{UclampAction, UclampValues, read_current_uclamp};
use crate::actions::{
    ActionPhase, RollbackToken, TaskIdentity, TuningAction,
    runner::{ActionRunPolicy, run_audited_action_with_audit_path},
};

type Originals = BTreeMap<u32, (u64, (u32, u32))>;

fn readback(original: &Originals, expected_min: Option<u32>) -> anyhow::Result<()> {
    for (tid, (ticks, (min, max))) in original {
        ensure!(
            crate::process_tree::process_starttime_at(Path::new("/proc"), *tid) == Some(*ticks),
            "acceptance task identity changed: {tid}"
        );
        let (now_min, now_max) = read_current_uclamp(*tid)?;
        ensure!(
            now_min == expected_min.unwrap_or(*min) && now_max == *max,
            "kernel uclamp readback differs for task {tid}: {now_min}/{now_max}"
        );
    }
    Ok(())
}

/// Scope must be a caller-owned disposable tree; the audit file must be absent.
pub fn verify_uclamp_lifecycle(
    tree_pid: u32,
    expected_starttime: u64,
    util_min: u32,
    audit_path: &Path,
) -> anyhow::Result<Value> {
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
        let values = read_current_uclamp(tid)?;
        ensure!(
            values.0 != util_min,
            "no observable uclamp change for task {tid}"
        );
        let ticks = crate::process_tree::process_starttime_at(Path::new("/proc"), tid)
            .context("task identity missing")?;
        original.insert(tid, (ticks, values));
    }
    let action = UclampAction {
        targets: snapshot
            .tasks
            .values()
            .map(TaskIdentity::from_task_info)
            .collect(),
        values: UclampValues {
            sched_util_min: Some(util_min),
            sched_util_max: None,
        },
    };
    let source = crate::daemon_policy::ActionSource::Cli;
    let mut dry_policy = ActionRunPolicy::dry_run(source);
    dry_policy.max_affected_tasks = Some(8);
    let dry = run_audited_action_with_audit_path(
        "uclamp-acceptance-dry-run",
        &action,
        dry_policy,
        audit_path,
    )?;
    ensure!(
        dry.state.affected_tasks == original.len() && !dry.state.applied,
        "dry-run scope mismatch"
    );
    readback(&original, None)?;
    let mut policy = ActionRunPolicy::apply_medium_risk(source, false);
    // The explicit medium-risk unlock the validation harness is granted.
    policy.policy = crate::daemon::DaemonPolicy::apply_medium_risk(source);
    policy.max_affected_tasks = Some(8);
    policy.max_total_duration = Some(std::time::Duration::from_secs(10));
    let applied =
        run_audited_action_with_audit_path("uclamp-acceptance-apply", &action, policy, audit_path)?;
    let token = applied
        .rollback
        .clone()
        .context("applied action omitted its rollback token")?;
    // Every assertion after the mutation stays in this closure so rollback always runs.
    let verified = (|| -> anyhow::Result<()> {
        let records = token
            .as_uclamp_restore()
            .context("rollback token is not a uclamp restore")?;
        ensure!(
            records.len() == original.len(),
            "durable rollback record count differs"
        );
        for record in records {
            let (ticks, (min, max)) = original
                .get(&record.tid())
                .context("unexpected task in rollback token")?;
            let identity = record.restore_identity();
            ensure!(
                identity.starttime_ticks == Some(*ticks)
                    && record.original_util_min == *min
                    && record.original_util_max == *max,
                "rollback token identity or original value differs"
            );
        }
        ensure!(
            applied.state.applied && applied.state.checked_tasks == original.len(),
            "action did not verify complete scope"
        );
        readback(&original, Some(util_min))
    })();
    let rollback = rollback_all(&action, &token);
    let restored = readback(&original, None);
    let mut events = crate::audit::read_audit_tail(audit_path, 100)?;
    let mut event = events.last().cloned().context("action audit missing")?;
    event.unix_nanos = crate::audit::unix_nanos_now();
    event.command = "uclamp-acceptance-rollback".into();
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
    readback(&original, None)?;
    Ok(json!({
        "target_pid": tree_pid,
        "target_starttime_ticks": expected_starttime,
        "tasks": original.iter().map(|(tid, (ticks, (min, max)))| json!({
            "tid": tid, "starttime_ticks": ticks,
            "original_min": min, "original_max": max, "applied_min": util_min,
        })).collect::<Vec<_>>(),
        "process_count": process_count,
        "audit_events": events.len(),
        "dry_run_unchanged": true,
        "durable_rollback_verified": true,
        "independent_kernel_restore_verified": true,
        "rollback_idempotent": true,
    }))
}

fn rollback_all(action: &UclampAction, token: &RollbackToken) -> anyhow::Result<()> {
    action.rollback(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_root_is_refused_before_any_kernel_mutation() {
        let root = crate::test_support::TestRoot::new("uclamp-acceptance-stale");
        let audit = root.join("audit.jsonl");
        let pid = std::process::id();
        let before = read_current_uclamp(pid).unwrap();
        assert!(verify_uclamp_lifecycle(pid, 0, 7, &audit).is_err());
        assert_eq!(read_current_uclamp(pid).unwrap(), before);
        assert!(!audit.exists());
    }
}
