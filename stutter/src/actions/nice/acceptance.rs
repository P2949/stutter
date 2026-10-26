//! Real-kernel acceptance of the nice action (see `task_lifecycle`).
use std::path::Path;

use anyhow::ensure;
use serde_json::Value;

use super::{NiceAction, NicePolicy, read_current_nice};
use crate::actions::{
    RollbackToken,
    runner::ActionRunPolicy,
    task_lifecycle::{LifecycleSpec, Originals, verify_task_lifecycle},
};

fn state(tid: u32) -> anyhow::Result<String> {
    Ok(read_current_nice(tid)?.to_string())
}

fn check_token(token: &RollbackToken, original: &Originals) -> anyhow::Result<()> {
    let records = token
        .as_nice_restore()
        .ok_or_else(|| anyhow::anyhow!("rollback token is not a nice restore"))?;
    ensure!(
        records.len() == original.len(),
        "durable rollback record count differs"
    );
    for record in records {
        let (ticks, before) = original
            .get(&record.tid())
            .ok_or_else(|| anyhow::anyhow!("unexpected task in rollback token"))?;
        ensure!(
            record.restore_identity().starttime_ticks == Some(*ticks)
                && record.original_nice.to_string() == *before,
            "rollback token identity or original value differs"
        );
    }
    Ok(())
}

/// Scope must be a caller-owned disposable tree; the audit file must be absent.
pub fn verify_nice_lifecycle(
    tree_pid: u32,
    expected_starttime: u64,
    nice: i32,
    audit_path: &Path,
) -> anyhow::Result<Value> {
    // The lifecycle un-demotes the tasks it demoted; see `raise_nice_rlimit_for_validation`.
    crate::syscall::raise_nice_rlimit_for_validation()?;
    let source = crate::daemon_policy::ActionSource::Cli;
    let mut apply_policy = ActionRunPolicy::apply_medium_risk(source, false);
    // The explicit medium-risk unlock the validation harness is granted.
    apply_policy.policy = crate::daemon::DaemonPolicy::apply_medium_risk(source);
    verify_task_lifecycle(LifecycleSpec {
        label: "nice",
        tree_pid,
        expected_starttime,
        audit_path,
        apply_policy,
        build: &|targets| {
            Ok(NiceAction {
                targets,
                nice,
                policy: NicePolicy::default(),
            })
        },
        read: &state,
        applied: &|_| Ok(nice.to_string()),
        check_token: &check_token,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_root_is_refused_before_any_kernel_mutation() {
        let root = crate::test_support::TestRoot::new("nice-acceptance-stale");
        let audit = root.join("audit.jsonl");
        let pid = std::process::id();
        let before = read_current_nice(pid).unwrap();
        assert!(verify_nice_lifecycle(pid, 0, 7, &audit).is_err());
        assert_eq!(read_current_nice(pid).unwrap(), before);
        assert!(!audit.exists());
    }
}
