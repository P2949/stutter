//! Independent procfs observations; no production action/restore reader is used.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
};

use anyhow::{Context, ensure};
use serde_json::{Value, json};
use stutter::api::autotune::AcceptanceMutation;

use super::super::start_ticks;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TaskMask {
    pub pid: u32,
    pub tid: u32,
    pub process_ticks: u64,
    pub task_ticks: u64,
    /// The mutated property as the kernel reports it: an allowed-CPU list or `min/max` uclamp.
    pub mask: String,
}

static SYSFS_FIXTURE: std::sync::Mutex<Option<std::path::PathBuf>> = std::sync::Mutex::new(None);

/// The sysfs fixture of the case being run (CPU power mutations); `None` for every other mutation.
pub(super) fn set_sysfs_fixture(root: Option<std::path::PathBuf>) {
    if let Ok(mut slot) = SYSFS_FIXTURE.lock() {
        *slot = root;
    }
}

fn sysfs_fixture() -> anyhow::Result<std::path::PathBuf> {
    SYSFS_FIXTURE
        .lock()
        .ok()
        .and_then(|slot| slot.clone())
        .context("no sysfs fixture for this case")
}

/// One task's property read straight from procfs.
fn read_property(mutation: AcceptanceMutation, pid: u32, tid: u32) -> anyhow::Result<String> {
    match mutation {
        AcceptanceMutation::Affinity => {
            let status = fs::read_to_string(format!("/proc/{pid}/task/{tid}/status"))?;
            Ok(status
                .lines()
                .find_map(|line| line.strip_prefix("Cpus_allowed_list:"))
                .context("task affinity missing")?
                .trim()
                .to_owned())
        }
        AcceptanceMutation::CpuPowerEpp => Ok(fs::read_to_string(sysfs_fixture()?.join(format!(
            "devices/system/cpu/cpu{tid}/cpufreq/energy_performance_preference"
        )))?
        .trim()
        .to_owned()),
        AcceptanceMutation::Cgroup => {
            let text = fs::read_to_string(format!("/proc/{pid}/task/{tid}/cgroup"))?;
            Ok(text
                .lines()
                .find_map(|line| line.strip_prefix("0::"))
                .context("cgroup v2 path missing")?
                .trim()
                .to_owned())
        }
        AcceptanceMutation::Nice => {
            let stat = fs::read_to_string(format!("/proc/{pid}/task/{tid}/stat"))?;
            Ok(stat
                .rsplit_once(')')
                .and_then(|(_, tail)| tail.split_whitespace().nth(16))
                .context("nice field missing")?
                .to_owned())
        }
        AcceptanceMutation::Uclamp => {
            let sched = fs::read_to_string(format!("/proc/{pid}/task/{tid}/sched"))?;
            let field = |key: &str| {
                sched
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        (name.trim() == key).then(|| value.trim().to_owned())
                    })
                    .with_context(|| format!("{key} missing"))
            };
            Ok(format!("{}/{}", field("uclamp.min")?, field("uclamp.max")?))
        }
    }
}

/// The property a task has once the mutation is applied, derived from what it had before.
pub(super) fn applied_value(
    mutation: AcceptanceMutation,
    original: &str,
    desired: u32,
) -> anyhow::Result<String> {
    Ok(match mutation {
        AcceptanceMutation::Affinity | AcceptanceMutation::Nice => desired.to_string(),
        AcceptanceMutation::CpuPowerEpp => "performance".to_owned(),
        // The tree moves into the `target` sibling of the leaf it started in.
        AcceptanceMutation::Cgroup => std::path::Path::new(original)
            .parent()
            .context("the tree does not run in a delegated cgroup subtree")?
            .join("target")
            .to_string_lossy()
            .into_owned(),
        AcceptanceMutation::Uclamp => {
            let (_, max) = original.split_once('/').context("uclamp value malformed")?;
            format!("{desired}/{max}")
        }
    })
}
pub(super) type Snapshot = BTreeMap<u32, TaskMask>;
pub(super) fn snapshot(
    mutation: AcceptanceMutation,
    root: u32,
    ticks: u64,
) -> anyhow::Result<Snapshot> {
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
            let mask = read_property(mutation, pid, tid)?;
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
    mutation: AcceptanceMutation,
    before: &Snapshot,
    current: &Snapshot,
    desired: u32,
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
        ensure!(
            now.mask == applied_value(mutation, &task.mask, desired)?,
            "unexpected property change for {tid}"
        );
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
/// The durable original state of every task, checked against what the harness itself read before
/// the mutation. Affinity keeps it in a restore file named by the token; uclamp inside the token.
pub(super) fn verify_saved(
    mutation: AcceptanceMutation,
    before: &Snapshot,
    token: &Value,
    saved_file: Option<&Value>,
    desired: u32,
) -> anyhow::Result<()> {
    match mutation {
        AcceptanceMutation::Affinity => verify_saved_affinity(
            before,
            saved_file.context("affinity restore file missing")?,
            &desired.to_string(),
        ),
        AcceptanceMutation::Uclamp => verify_saved_uclamp(before, token),
        AcceptanceMutation::Nice => verify_saved_nice(before, token),
        AcceptanceMutation::Cgroup => verify_saved_cgroup(before, token),
        AcceptanceMutation::CpuPowerEpp => verify_saved_epp(before, token),
    }
}

fn verify_saved_epp(before: &Snapshot, token: &Value) -> anyhow::Result<()> {
    ensure!(
        token["kind"] == "CpuPowerRestore",
        "cpu power token kind differs"
    );
    let records = token["records"]
        .as_array()
        .context("cpu power token records missing")?;
    ensure!(records.len() == before.len(), "token record count differs");
    let fixture = sysfs_fixture()?;
    for task in before.values() {
        let path = fixture.join(format!(
            "devices/system/cpu/cpu{}/cpufreq/energy_performance_preference",
            task.tid
        ));
        let record = records
            .iter()
            .find(|record| record["path"].as_str() == path.to_str())
            .with_context(|| format!("no token record for the fixture of task {}", task.tid))?;
        ensure!(
            record["original_value"].as_str() == Some(task.mask.as_str()),
            "token original value differs for {}",
            task.tid
        );
        ensure!(
            record["identity"]["attribute"] == "energy_performance_preference"
                && record["identity"]["boot_id"].is_string(),
            "token cpufreq identity incomplete for {}",
            task.tid
        );
    }
    Ok(())
}

fn verify_saved_cgroup(before: &Snapshot, token: &Value) -> anyhow::Result<()> {
    ensure!(
        token["kind"] == "CgroupRestore",
        "cgroup token kind differs"
    );
    let records = token["records"]
        .as_array()
        .context("cgroup token records missing")?;
    ensure!(records.len() == before.len(), "token task count differs");
    let mut seen = BTreeSet::new();
    for record in records {
        let identity = &record["identity"];
        let tid = record["tid"]
            .as_u64()
            .and_then(|tid| u32::try_from(tid).ok())
            .context("token TID invalid")?;
        let task = before.get(&tid).context("token includes unexpected task")?;
        ensure!(seen.insert(tid), "duplicate token task");
        ensure!(
            identity["process_pid"].as_u64() == Some(u64::from(task.pid))
                && identity["starttime_ticks"].as_u64() == Some(task.task_ticks),
            "token identity differs for {tid}"
        );
        ensure!(
            record["original_cgroup"].as_str() == Some(task.mask.as_str())
                && record["original_cgroup_id"].as_u64().is_some(),
            "token original cgroup or its identity differs for {tid}"
        );
    }
    Ok(())
}

fn verify_saved_nice(before: &Snapshot, token: &Value) -> anyhow::Result<()> {
    ensure!(token["kind"] == "NiceRestore", "nice token kind differs");
    let records = token["records"]
        .as_array()
        .context("nice token records missing")?;
    ensure!(records.len() == before.len(), "token task count differs");
    let mut seen = BTreeSet::new();
    for record in records {
        let identity = &record["identity"];
        let tid = record["tid"]
            .as_u64()
            .and_then(|tid| u32::try_from(tid).ok())
            .context("token TID invalid")?;
        let task = before.get(&tid).context("token includes unexpected task")?;
        ensure!(seen.insert(tid), "duplicate token task");
        ensure!(
            identity["process_pid"].as_u64() == Some(u64::from(task.pid))
                && identity["starttime_ticks"].as_u64() == Some(task.task_ticks),
            "token identity differs for {tid}"
        );
        ensure!(
            record["original_nice"]
                .as_i64()
                .map(|nice| nice.to_string())
                .as_deref()
                == Some(task.mask.as_str()),
            "token original value differs for {tid}"
        );
    }
    Ok(())
}

fn verify_saved_uclamp(before: &Snapshot, token: &Value) -> anyhow::Result<()> {
    ensure!(
        token["kind"] == "UclampRestore",
        "uclamp token kind differs"
    );
    let records = token["records"]
        .as_array()
        .context("uclamp token records missing")?;
    ensure!(records.len() == before.len(), "token task count differs");
    let mut seen = BTreeSet::new();
    for record in records {
        let identity = &record["identity"];
        let tid = record["tid"]
            .as_u64()
            .and_then(|tid| u32::try_from(tid).ok())
            .context("token TID invalid")?;
        let task = before.get(&tid).context("token includes unexpected task")?;
        ensure!(seen.insert(tid), "duplicate token task");
        ensure!(
            identity["process_pid"].as_u64() == Some(u64::from(task.pid))
                && identity["starttime_ticks"].as_u64() == Some(task.task_ticks),
            "token identity differs for {tid}"
        );
        let original = format!(
            "{}/{}",
            record["original_util_min"], record["original_util_max"]
        );
        ensure!(
            original == task.mask,
            "token original value differs for {tid}"
        );
    }
    Ok(())
}

fn verify_saved_affinity(before: &Snapshot, saved: &Value, desired: &str) -> anyhow::Result<()> {
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
        let affinity = AcceptanceMutation::Affinity;
        assert_eq!(changed(affinity, &task("0-3"), &task("0"), 0).unwrap(), 1);
        assert!(changed(affinity, &task("0-3"), &task("1"), 0).is_err());
        let mut reused = task("0");
        reused.get_mut(&1).unwrap().task_ticks = 12;
        assert!(changed(affinity, &task("0-3"), &reused, 0).is_err());
        assert!(changed(affinity, &task("0-3"), &BTreeMap::new(), 0).is_err());
    }

    #[test]
    fn uclamp_change_must_be_exactly_the_requested_minimum() {
        let uclamp = AcceptanceMutation::Uclamp;
        assert_eq!(
            changed(uclamp, &task("0/1024"), &task("256/1024"), 256).unwrap(),
            1
        );
        assert!(changed(uclamp, &task("0/1024"), &task("128/1024"), 256).is_err());
        assert!(changed(uclamp, &task("0/1024"), &task("256/512"), 256).is_err());
        assert_eq!(
            changed(uclamp, &task("0/1024"), &task("0/1024"), 256).unwrap(),
            0
        );
    }
}
