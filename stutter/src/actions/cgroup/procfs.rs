use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::Context;

use super::{fs_io::normalize_cgroup_path, model::CgroupTargetSnapshot};
use crate::actions::{ActionBoundaryError, ActionWarning, TaskIdentity};

pub(super) fn read_target_snapshot_at(
    proc_root: &Path,
    target: &TaskIdentity,
) -> anyhow::Result<CgroupTargetSnapshot> {
    if target.tid == 0 {
        return Err(ActionBoundaryError::InvalidTargetTid {
            action_kind: "cgroup",
            tid: target.tid.as_u32(),
        }
        .into());
    }

    let stat_path = proc_root.join(target.tid.to_string()).join("stat");
    let stat = fs::read_to_string(&stat_path).with_context(|| {
        format!(
            "target task does not exist or stat is unreadable: {}",
            stat_path.display()
        )
    })?;
    let starttime_ticks = parse_stat_starttime(&stat)
        .with_context(|| format!("failed to parse starttime from {}", stat_path.display()))?;

    if let Some(expected_starttime) = target.starttime_ticks
        && expected_starttime != starttime_ticks
    {
        return Err(ActionBoundaryError::TargetIdentityMismatch {
            action_kind: "cgroup",
            tid: target.tid.as_u32(),
            expected_starttime,
            actual_starttime: starttime_ticks,
        }
        .into());
    }

    let comm_path = proc_root.join(target.tid.to_string()).join("comm");
    let comm = fs::read_to_string(comm_path)
        .ok()
        .map(|comm| comm.trim().to_owned())
        .filter(|comm| !comm.is_empty());
    let exe = fs::read_link(proc_root.join(target.tid.to_string()).join("exe")).ok();

    let original_cgroup = read_proc_cgroup_path_at(proc_root, target.tid.as_u32())
        .with_context(|| format!("failed to read cgroup path for tid={}", target.tid))?;

    Ok(CgroupTargetSnapshot {
        tid: target.tid.as_u32(),
        process_pid: target.process_pid.map(|pid| pid.as_u32()),
        comm,
        starttime_ticks: Some(starttime_ticks),
        exe,
        original_cgroup,
    })
}

pub(super) fn identity_warnings(
    target: &TaskIdentity,
    snapshot: &CgroupTargetSnapshot,
) -> Vec<ActionWarning> {
    let mut warnings = Vec::new();

    if let (Some(expected_comm), Some(actual_comm)) = (&target.comm, &snapshot.comm)
        && expected_comm != actual_comm
    {
        warnings.push(ActionWarning {
         message: format!(
             "target tid={} comm changed from {:?} to {:?}; continuing because starttime matched or was not provided",
             target.tid, expected_comm, actual_comm
         ),
     });
    }

    if target.process_pid.is_none() {
        warnings.push(ActionWarning {
            message: format!(
                "target tid={} has no process_pid identity; rollback will use tid only",
                target.tid
            ),
        });
    }

    warnings
}

pub(super) fn parse_stat_starttime(stat: &str) -> anyhow::Result<u64> {
    let close_paren = stat
        .rfind(')')
        .context("stat line does not contain closing comm parenthesis")?;
    let fields = stat[close_paren + 1..]
        .split_whitespace()
        .collect::<Vec<_>>();

    let starttime_ticks = fields
        .get(19)
        .context("stat line missing starttime field")?
        .parse::<u64>()
        .context("invalid starttime field")?;

    Ok(starttime_ticks)
}

pub(super) fn read_proc_cgroup_path_at(proc_root: &Path, tid: u32) -> anyhow::Result<PathBuf> {
    let cgroup_path = proc_root.join(tid.to_string()).join("cgroup");
    let contents = fs::read_to_string(&cgroup_path)
        .with_context(|| format!("failed to read {}", cgroup_path.display()))?;

    let path = contents
        .lines()
        .filter_map(|line| line.rsplit_once(':').map(|(_, path)| path.trim()))
        .filter(|path| !path.is_empty())
        .max_by_key(|path| path.len())
        .context("proc cgroup file did not contain a cgroup v2 path")?;

    normalize_cgroup_path(Path::new(path))
}

pub(super) fn task_exists(proc_root: &Path, tid: u32) -> bool {
    proc_root.join(tid.to_string()).join("stat").is_file()
}
