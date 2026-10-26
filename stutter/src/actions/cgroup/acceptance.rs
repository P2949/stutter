//! Real-kernel acceptance of cgroup placement (see `task_lifecycle`). The owned tree runs in a
//! leaf of a subtree the runner delegated to this user; the lifecycle moves it into a sibling
//! `target` and back.
use std::path::Path;

use anyhow::{Context, ensure};
use serde_json::Value;

use super::{CgroupPlacementAction, CgroupPlacementTarget};
use crate::actions::{
    RollbackToken, TaskIdentity,
    runner::ActionRunPolicy,
    task_lifecycle::{LifecycleSpec, Originals, verify_task_lifecycle},
};

fn state(tid: u32) -> anyhow::Result<String> {
    let text = std::fs::read_to_string(format!("/proc/{tid}/cgroup"))?;
    text.lines()
        .find_map(|line| line.strip_prefix("0::"))
        .map(|path| path.trim().to_owned())
        .context("no cgroup v2 path")
}

fn sibling_target(original: &str) -> anyhow::Result<String> {
    let parent = Path::new(original)
        .parent()
        .filter(|parent| parent.components().count() > 1)
        .context("the tree does not run in a delegated cgroup subtree")?;
    Ok(parent.join("target").to_string_lossy().into_owned())
}

fn check_token(token: &RollbackToken, original: &Originals) -> anyhow::Result<()> {
    let (records, cpuset) = token
        .as_cgroup_restore()
        .context("rollback token is not a cgroup restore")?;
    ensure!(cpuset.is_none(), "unexpected cpuset change in the token");
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
                && record.original_cgroup.to_string_lossy() == *before
                && record.original_cgroup_id.is_some(),
            "rollback token identity or original cgroup differs"
        );
    }
    Ok(())
}

/// Scope must be a caller-owned disposable tree in a delegated cgroup; the audit file must be new.
pub fn verify_cgroup_lifecycle(
    tree_pid: u32,
    expected_starttime: u64,
    audit_path: &Path,
) -> anyhow::Result<Value> {
    let source = crate::daemon_policy::ActionSource::Cli;
    let mut apply_policy = ActionRunPolicy::apply_medium_risk(source, false);
    // The explicit medium-risk unlock the validation harness is granted.
    apply_policy.policy = crate::daemon::DaemonPolicy::apply_medium_risk(source);
    let target = sibling_target(&state(tree_pid)?)?;
    std::fs::create_dir_all(Path::new("/sys/fs/cgroup").join(target.trim_start_matches('/')))
        .context("cannot create the target cgroup inside the delegated subtree")?;
    verify_task_lifecycle(LifecycleSpec {
        label: "cgroup",
        tree_pid,
        expected_starttime,
        audit_path,
        apply_policy,
        build: &|targets: Vec<TaskIdentity>| CgroupPlacementAction {
            cgroup_root: "/sys/fs/cgroup".into(),
            target_cgroup: target.clone().into(),
            targets: targets
                .into_iter()
                .map(|identity| CgroupPlacementTarget {
                    identity,
                    class: crate::process_tree::TaskClass::Game,
                })
                .collect(),
            cpuset_cpus: None,
            cpuset_mems: None,
        },
        read: &state,
        applied: &|original| sibling_target(original),
        check_token: &check_token,
    })
}
