//! Independent procfs observations; no production action/restore reader is used.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
};

use anyhow::{Context, ensure};
use serde_json::{Value, json};

use super::super::start_ticks;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TaskMask {
    pub pid: u32,
    pub tid: u32,
    pub process_ticks: u64,
    pub task_ticks: u64,
    pub mask: String,
}
pub(super) type Snapshot = BTreeMap<u32, TaskMask>;
pub(super) fn snapshot(root: u32, ticks: u64) -> anyhow::Result<Snapshot> {
    ensure!(start_ticks(root)? == ticks, "root process identity changed");
    let mut pids = fs::read_to_string(format!("/proc/{root}/task/{root}/children"))?
        .split_whitespace()
        .map(str::parse::<u32>)
        .collect::<Result<BTreeSet<_>, _>>()?;
    ensure!(pids.len() == 2, "expected two direct helper children");
    pids.insert(root);
    let mut tasks = BTreeMap::new();
    for pid in pids {
        let process_ticks = start_ticks(pid)?;
        for entry in fs::read_dir(format!("/proc/{pid}/task"))? {
            let tid = entry?.file_name().to_string_lossy().parse::<u32>()?;
            let task_ticks = start_ticks(tid)?;
            let status = fs::read_to_string(format!("/proc/{pid}/task/{tid}/status"))?;
            let mask = status
                .lines()
                .find_map(|line| line.strip_prefix("Cpus_allowed_list:"))
                .context("task affinity missing")?
                .trim()
                .to_owned();
            ensure!(!mask.is_empty(), "empty task mask");
            tasks.insert(
                tid,
                TaskMask {
                    pid,
                    tid,
                    process_ticks,
                    task_ticks,
                    mask,
                },
            );
        }
    }
    ensure!(
        tasks.len() == 5,
        "expected five helper tasks, got {}",
        tasks.len()
    );
    ensure!(
        start_ticks(root)? == ticks,
        "root identity changed during snapshot"
    );
    Ok(tasks)
}
pub(super) fn changed(
    before: &Snapshot,
    current: &Snapshot,
    desired: &str,
) -> anyhow::Result<usize> {
    ensure!(before.keys().eq(current.keys()), "target task set changed");
    let mut changed = 0;
    for (tid, task) in before {
        let now = &current[tid];
        ensure!(
            task.pid == now.pid
                && task.process_ticks == now.process_ticks
                && task.task_ticks == now.task_ticks,
            "target identity changed for {tid}"
        );
        if now.mask == task.mask {
            continue;
        }
        ensure!(now.mask == desired, "unexpected mask change for {tid}");
        changed += 1;
    }
    Ok(changed)
}
pub(super) fn value(snapshot: &Snapshot) -> Value {
    Value::Array(snapshot.values().map(|task| json!({"pid":task.pid,"tid":task.tid,"process_starttime_ticks":task.process_ticks,"task_starttime_ticks":task.task_ticks,"mask":task.mask})).collect())
}
pub(super) fn stopped(before: &Snapshot) -> bool {
    before.values().all(|task| {
        let stat = match fs::read_to_string(format!("/proc/{}/stat", task.tid)) {
            Ok(stat) => stat,
            Err(error) => return error.kind() == std::io::ErrorKind::NotFound,
        };
        let Some((_, tail)) = stat.rsplit_once(')') else {
            return false;
        };
        let fields = tail.split_whitespace().collect::<Vec<_>>();
        let ticks = fields.get(19).and_then(|field| field.parse::<u64>().ok());
        ticks.is_some_and(|ticks| ticks != task.task_ticks) || fields.first() == Some(&"Z")
    })
}
pub(super) fn verify_saved(before: &Snapshot, saved: &Value, desired: &str) -> anyhow::Result<()> {
    ensure!(saved["schema_version"] == 1, "restore schema differs");
    ensure!(
        saved["nice_records"].as_array().is_some_and(Vec::is_empty)
            && saved["ionice_records"]
                .as_array()
                .is_some_and(Vec::is_empty),
        "unexpected priority restoration scope"
    );
    let records = saved["affinity_records"]
        .as_array()
        .context("restore affinity records missing")?;
    ensure!(records.len() == before.len(), "restore task count differs");
    let mut seen = BTreeSet::new();
    for record in records {
        let tid = record["tid"]
            .as_u64()
            .and_then(|tid| u32::try_from(tid).ok())
            .context("restore TID invalid")?;
        let task = before
            .get(&tid)
            .context("restore includes unexpected task")?;
        ensure!(seen.insert(tid), "duplicate restore task");
        ensure!(
            record["process_pid"].as_u64() == Some(u64::from(task.pid))
                && record["process_starttime_ticks"].as_u64() == Some(task.process_ticks)
                && record["task_starttime_ticks"].as_u64() == Some(task.task_ticks),
            "restore identity differs"
        );
        ensure!(
            record["original_mask"] == task.mask && record["applied_mask"] == desired,
            "restore mask provenance differs"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn task(mask: &str) -> Snapshot {
        BTreeMap::from([(
            1,
            TaskMask {
                pid: 1,
                tid: 1,
                process_ticks: 10,
                task_ticks: 11,
                mask: mask.into(),
            },
        )])
    }
    #[test]
    fn independent_readback_rejects_replacement_unexpected_masks_and_task_loss() {
        assert_eq!(changed(&task("0-3"), &task("0"), "0").unwrap(), 1);
        assert!(changed(&task("0-3"), &task("1"), "0").is_err());
        let mut reused = task("0");
        reused.get_mut(&1).unwrap().task_ticks = 12;
        assert!(changed(&task("0-3"), &reused, "0").is_err());
        assert!(changed(&task("0-3"), &BTreeMap::new(), "0").is_err());
    }
}
