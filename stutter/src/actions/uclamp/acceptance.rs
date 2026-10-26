//! Real-kernel acceptance of the uclamp action (see `task_lifecycle`).
use std::path::Path;

use anyhow::{Context, ensure};
use serde_json::Value;

use super::{UclampAction, UclampValues, read_current_uclamp};
use crate::actions::{
    RollbackToken,
    runner::ActionRunPolicy,
    task_lifecycle::{LifecycleSpec, Originals, verify_task_lifecycle},
};

fn state(tid: u32) -> anyhow::Result<String> {
    let (min, max) = read_current_uclamp(tid)?;
    Ok(format!("{min}/{max}"))
}

fn with_min(original: &str, util_min: u32) -> anyhow::Result<String> {
    let (_, max) = original.split_once('/').context("uclamp state malformed")?;
    Ok(format!("{util_min}/{max}"))
}

fn check_token(token: &RollbackToken, original: &Originals) -> anyhow::Result<()> {
    let records = token
        .as_uclamp_restore()
        .context("rollback token is not a uclamp restore")?;
    ensure!(
        records.len() == original.len(),
        "durable rollback record count differs"
    );
    for record in records {
        let (ticks, before) = original
            .get(&record.tid())
            .context("unexpected task in rollback token")?;
        ensure!(
            record.restore_identity().starttime_ticks == Some(*ticks)
                && format!("{}/{}", record.original_util_min, record.original_util_max) == *before,
            "rollback token identity or original value differs"
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
    let source = crate::daemon_policy::ActionSource::Cli;
    let mut apply_policy = ActionRunPolicy::apply_medium_risk(source, false);
    // The explicit medium-risk unlock the validation harness is granted.
    apply_policy.policy = crate::daemon::DaemonPolicy::apply_medium_risk(source);
    verify_task_lifecycle(LifecycleSpec {
        label: "uclamp",
        tree_pid,
        expected_starttime,
        audit_path,
        apply_policy,
        build: &|targets| UclampAction {
            targets,
            values: UclampValues {
                sched_util_min: Some(util_min),
                sched_util_max: None,
            },
        },
        read: &state,
        applied: &|original| with_min(original, util_min),
        check_token: &check_token,
    })
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
