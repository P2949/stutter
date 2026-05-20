//! Target resolution helpers for apply-low-risk command tests.
//!
//! Owns selector validation and procfs target-tree discovery. It must not choose candidates,
//! compare experiments, or perform audited action application.

use std::{fs, path::Path};

use anyhow::Context;

pub fn resolve_one_target_tree_pid(
    tree_pid: Option<u32>,
    watch_process: Option<&str>,
) -> anyhow::Result<u32> {
    resolve_one_target_tree_pid_at(Path::new("/proc"), tree_pid, watch_process)
}

pub fn resolve_one_target_tree_pid_at(
    proc_root: &Path,
    tree_pid: Option<u32>,
    watch_process: Option<&str>,
) -> anyhow::Result<u32> {
    match (tree_pid, watch_process) {
        (Some(_), Some(_)) => {
            anyhow::bail!(
                "apply-low-risk requires exactly one target selector; pass either --tree-pid or --watch-process, not both"
            )
        }
        (None, None) => {
            anyhow::bail!(
                "apply-low-risk requires exactly one target selector; pass --tree-pid or --watch-process"
            )
        }
        (Some(pid), None) => {
            validate_one_live_tree_at(proc_root, pid)?;
            Ok(pid)
        }
        (None, Some(comm)) => {
            let matches = process_roots_by_comm_at(proc_root, comm)?;
            match matches.as_slice() {
                [pid] => {
                    validate_one_live_tree_at(proc_root, *pid)?;
                    Ok(*pid)
                }
                [] => anyhow::bail!(
                    "apply-low-risk could not find active target process with comm '{}'",
                    comm
                ),
                _ => anyhow::bail!(
                    "apply-low-risk requires one active target tree; comm '{}' matched {} processes: {:?}",
                    comm,
                    matches.len(),
                    matches
                ),
            }
        }
    }
}

fn validate_one_live_tree_at(proc_root: &Path, tree_pid: u32) -> anyhow::Result<()> {
    if tree_pid == 0 {
        anyhow::bail!("tree pid must be greater than zero");
    }

    let tree_pids = [tree_pid];
    let snapshot = crate::process_tree::target_snapshot(
        crate::process_tree::TargetSnapshotInput::default()
            .proc_root(proc_root)
            .tree_pids(&tree_pids),
    );

    if !snapshot.process_roots.contains(&tree_pid) || snapshot.tasks.is_empty() {
        anyhow::bail!(
            "apply-low-risk target tree {} is not active or has no tasks",
            tree_pid
        );
    }

    Ok(())
}

fn process_roots_by_comm_at(proc_root: &Path, comm: &str) -> anyhow::Result<Vec<u32>> {
    let mut matches = Vec::new();

    for entry in fs::read_dir(proc_root)
        .with_context(|| format!("failed to read proc root {}", proc_root.display()))?
    {
        let entry = entry?;
        let Some(file_name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Ok(pid) = file_name.parse::<u32>() else {
            continue;
        };

        let comm_path = entry.path().join("comm");
        let Ok(found_comm) = fs::read_to_string(comm_path) else {
            continue;
        };

        if found_comm.trim() == comm {
            matches.push(pid);
        }
    }

    matches.sort_unstable();
    Ok(matches)
}
