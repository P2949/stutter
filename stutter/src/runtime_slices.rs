use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

use crate::{
    metrics::{RuntimeSliceRecord, RuntimeSliceSource},
    process_tree::TaskInfo,
};

#[derive(Debug)]
pub struct RuntimeSliceSampler {
    proc_root: PathBuf,
    clock_ticks_per_second: u64,
    previous: BTreeMap<u32, RuntimeSliceSnapshot>,
}

#[derive(Clone, Debug)]
pub struct RuntimeSliceSnapshot {
    task: u32,
    process_pid: u32,
    process_starttime_ticks: Option<u64>,
    task_starttime_ticks: Option<u64>,
    source: RuntimeSliceSource,
    runtime_ns: u64,
    runqueue_wait_ns: Option<u64>,
    timeslices: Option<u64>,
    user_runtime_ns: Option<u64>,
    system_runtime_ns: Option<u64>,
}

#[derive(Default, Debug)]
pub struct RuntimeSliceBatch {
    pub records: Vec<RuntimeSliceRecord>,
    pub scanned_tasks: usize,
    pub skipped_tasks: usize,
    pub read_errors: u64,
    pub schedstat_available: bool,
}

impl RuntimeSliceSampler {
    pub fn new() -> Self {
        Self {
            proc_root: PathBuf::from("/proc"),
            clock_ticks_per_second: clock_ticks_per_second(),
            previous: BTreeMap::new(),
        }
    }

    #[cfg(test)]
    pub fn with_proc_root(proc_root: PathBuf) -> Self {
        Self {
            proc_root,
            clock_ticks_per_second: clock_ticks_per_second(),
            previous: BTreeMap::new(),
        }
    }

    pub fn collect(
        &mut self,
        tasks: &[TaskInfo],
        elapsed_ms: u64,
        interval_ms: u64,
        max_tasks: usize,
    ) -> RuntimeSliceBatch {
        let mut batch = RuntimeSliceBatch {
            skipped_tasks: tasks.len().saturating_sub(max_tasks),
            ..Default::default()
        };
        let mut active_tids = BTreeSet::new();

        for task in tasks.iter().take(max_tasks) {
            active_tids.insert(task.tid);
            batch.scanned_tasks += 1;

            let snapshot = match read_snapshot(&self.proc_root, self.clock_ticks_per_second, task) {
                Ok(snapshot) => snapshot,
                Err(err) => {
                    batch.read_errors += 1;
                    log::debug!(
                        "runtime_slice_read_failed tid={} pid={} err={err:#}",
                        task.tid,
                        task.process_pid
                    );
                    self.previous.remove(&task.tid);
                    continue;
                }
            };

            batch.schedstat_available |= snapshot.source == RuntimeSliceSource::ProcSchedstat;

            let Some(previous) = self.previous.insert(task.tid, snapshot.clone()) else {
                continue;
            };

            if !same_task_identity(&previous, &snapshot) {
                continue;
            }

            if let Some(record) = build_record(task, &previous, &snapshot, elapsed_ms, interval_ms)
            {
                batch.records.push(record);
            }
        }

        self.previous.retain(|tid, _| active_tids.contains(tid));

        batch
    }
}

impl Default for RuntimeSliceSampler {
    fn default() -> Self {
        Self::new()
    }
}

fn same_task_identity(left: &RuntimeSliceSnapshot, right: &RuntimeSliceSnapshot) -> bool {
    if left.task != right.task || left.process_pid != right.process_pid {
        return false;
    }

    if let (Some(left_process), Some(right_process)) =
        (left.process_starttime_ticks, right.process_starttime_ticks)
        && left_process != right_process
    {
        return false;
    }

    if let (Some(left_task), Some(right_task)) =
        (left.task_starttime_ticks, right.task_starttime_ticks)
        && left_task != right_task
    {
        return false;
    }

    true
}

fn read_snapshot(
    proc_root: &Path,
    clock_ticks_per_second: u64,
    task: &TaskInfo,
) -> Result<RuntimeSliceSnapshot> {
    let task_dir = proc_root
        .join(task.process_pid.to_string())
        .join("task")
        .join(task.tid.to_string());
    let schedstat_path = task_dir.join("schedstat");

    if let Ok(contents) = fs::read_to_string(&schedstat_path) {
        let (runtime_ns, wait_ns, timeslices) = parse_schedstat(&contents)
            .with_context(|| format!("malformed {}", schedstat_path.display()))?;
        return Ok(RuntimeSliceSnapshot {
            task: task.tid,
            process_pid: task.process_pid,
            process_starttime_ticks: task.process_starttime_ticks,
            task_starttime_ticks: task.task_starttime_ticks,
            source: RuntimeSliceSource::ProcSchedstat,
            runtime_ns,
            runqueue_wait_ns: Some(wait_ns),
            timeslices: Some(timeslices),
            user_runtime_ns: None,
            system_runtime_ns: None,
        });
    }

    let stat_path = task_dir.join("stat");
    let contents = fs::read_to_string(&stat_path)
        .with_context(|| format!("failed to read {}", stat_path.display()))?;
    let (user_ticks, system_ticks) =
        parse_proc_stat_runtime_ticks(&contents).context("malformed proc stat")?;
    let user_runtime_ns = ticks_to_ns(user_ticks, clock_ticks_per_second);
    let system_runtime_ns = ticks_to_ns(system_ticks, clock_ticks_per_second);

    Ok(RuntimeSliceSnapshot {
        task: task.tid,
        process_pid: task.process_pid,
        process_starttime_ticks: task.process_starttime_ticks,
        task_starttime_ticks: task.task_starttime_ticks,
        source: RuntimeSliceSource::ProcStatFallback,
        runtime_ns: user_runtime_ns.saturating_add(system_runtime_ns),
        runqueue_wait_ns: None,
        timeslices: None,
        user_runtime_ns: Some(user_runtime_ns),
        system_runtime_ns: Some(system_runtime_ns),
    })
}

fn build_record(
    task: &TaskInfo,
    previous: &RuntimeSliceSnapshot,
    snapshot: &RuntimeSliceSnapshot,
    elapsed_ms: u64,
    interval_ms: u64,
) -> Option<RuntimeSliceRecord> {
    let runtime_delta_ns = snapshot.runtime_ns.saturating_sub(previous.runtime_ns);
    let wait_delta_ns = snapshot
        .runqueue_wait_ns
        .zip(previous.runqueue_wait_ns)
        .map(|(current, previous)| current.saturating_sub(previous));
    let timeslices_delta = snapshot
        .timeslices
        .zip(previous.timeslices)
        .map(|(current, previous)| current.saturating_sub(previous));

    let user_runtime_delta_ns = snapshot
        .user_runtime_ns
        .zip(previous.user_runtime_ns)
        .map(|(current, previous)| current.saturating_sub(previous));
    let system_runtime_delta_ns = snapshot
        .system_runtime_ns
        .zip(previous.system_runtime_ns)
        .map(|(current, previous)| current.saturating_sub(previous));

    let interval_ns = interval_ms.saturating_mul(1_000_000);
    let runtime_ratio = ratio(runtime_delta_ns, interval_ns);
    let wait_ratio = wait_delta_ns.and_then(|wait| ratio(wait, interval_ns));
    let avg_runtime_per_slice_ns =
        timeslices_delta.and_then(|slices| nonzero_div(runtime_delta_ns, slices));
    let avg_wait_per_slice_ns = wait_delta_ns
        .zip(timeslices_delta)
        .and_then(|(wait, slices)| nonzero_div(wait, slices));

    Some(RuntimeSliceRecord {
        elapsed_ms,
        task: task.tid,
        process_pid: (task.process_pid != 0).then_some(task.process_pid),
        class: task.class,
        comm: task.comm.clone(),
        process_comm: task.process_comm.clone(),
        source: snapshot.source,
        interval_ms,
        runtime_delta_ns,
        runqueue_wait_delta_ns: wait_delta_ns,
        timeslices_delta,
        user_runtime_delta_ns,
        system_runtime_delta_ns,
        runtime_ratio,
        wait_ratio,
        avg_runtime_per_slice_ns,
        avg_wait_per_slice_ns,
        valid: true,
        unavailable_reason: None,
    })
}

fn ratio(numerator: u64, denominator: u64) -> Option<f64> {
    (denominator > 0).then(|| numerator as f64 / denominator as f64)
}

fn nonzero_div(numerator: u64, denominator: u64) -> Option<u64> {
    (denominator > 0).then(|| numerator / denominator)
}

fn parse_schedstat(contents: &str) -> Result<(u64, u64, u64)> {
    let mut fields = contents.split_whitespace();
    let runtime_ns = fields
        .next()
        .context("missing runtime_ns")?
        .parse::<u64>()
        .context("invalid runtime_ns")?;
    let wait_ns = fields
        .next()
        .context("missing wait_ns")?
        .parse::<u64>()
        .context("invalid wait_ns")?;
    let timeslices = fields
        .next()
        .context("missing timeslices")?
        .parse::<u64>()
        .context("invalid timeslices")?;
    Ok((runtime_ns, wait_ns, timeslices))
}

fn parse_proc_stat_runtime_ticks(stat: &str) -> Result<(u64, u64)> {
    let end_comm = stat.rfind(')').context("missing comm terminator")?;
    let after = stat
        .get(end_comm + 2..)
        .context("missing fields after comm")?;
    let fields: Vec<&str> = after.split_whitespace().collect();

    let utime = fields
        .get(11)
        .context("missing utime")?
        .parse::<u64>()
        .context("invalid utime")?;
    let stime = fields
        .get(12)
        .context("missing stime")?
        .parse::<u64>()
        .context("invalid stime")?;

    Ok((utime, stime))
}

fn ticks_to_ns(ticks: u64, clock_ticks_per_second: u64) -> u64 {
    if clock_ticks_per_second == 0 {
        return 0;
    }
    ((ticks as u128).saturating_mul(1_000_000_000) / clock_ticks_per_second as u128) as u64
}

fn clock_ticks_per_second() -> u64 {
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if ticks > 0 { ticks as u64 } else { 100 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process_tree::TaskClass;

    fn task_info(tid: u32, process_pid: u32) -> TaskInfo {
        TaskInfo {
            tid,
            process_pid,
            process_ppid: 1,
            comm: "GameThread".to_owned(),
            process_comm: std::sync::Arc::<str>::from("Game.exe"),
            process_starttime_ticks: Some(10),
            task_starttime_ticks: Some(20),
            exe_dev: None,
            exe_ino: None,
            class: TaskClass::Game,
            sched_policy: None,
            from_cgroup: false,
        }
    }

    fn write_task_file(root: &Path, pid: u32, tid: u32, name: &str, contents: &str) {
        let task_dir = root
            .join(pid.to_string())
            .join("task")
            .join(tid.to_string());
        fs::create_dir_all(&task_dir).unwrap();
        fs::write(task_dir.join(name), contents).unwrap();
    }

    #[test]
    fn parses_schedstat() {
        let parsed = parse_schedstat("1000 2000 3\n").unwrap();
        assert_eq!(parsed, (1000, 2000, 3));
    }

    #[test]
    fn computes_deltas_and_ignores_first_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let task = task_info(101, 100);
        write_task_file(dir.path(), 100, 101, "schedstat", "1000 2000 4\n");

        let mut sampler = RuntimeSliceSampler::with_proc_root(dir.path().to_path_buf());
        let first = sampler.collect(std::slice::from_ref(&task), 1000, 1000, 256);
        assert!(first.records.is_empty());

        write_task_file(dir.path(), 100, 101, "schedstat", "501000000 102000000 9\n");
        let second = sampler.collect(std::slice::from_ref(&task), 2000, 1000, 256);
        assert_eq!(second.records.len(), 1);
        let record = &second.records[0];
        assert_eq!(record.runtime_delta_ns, 500_999_000);
        assert_eq!(record.runqueue_wait_delta_ns, Some(101_998_000));
        assert_eq!(record.timeslices_delta, Some(5));
        assert_eq!(record.avg_runtime_per_slice_ns, Some(100_199_800));
        assert!(record.runtime_ratio.unwrap() > 0.50);
        assert!(second.schedstat_available);
    }

    #[test]
    fn caps_max_scanned_tasks() {
        let dir = tempfile::tempdir().unwrap();
        let tasks = vec![task_info(101, 100), task_info(102, 100)];
        write_task_file(dir.path(), 100, 101, "schedstat", "1000 2000 4\n");
        write_task_file(dir.path(), 100, 102, "schedstat", "1000 2000 4\n");

        let mut sampler = RuntimeSliceSampler::with_proc_root(dir.path().to_path_buf());
        let batch = sampler.collect(&tasks, 1000, 1000, 1);

        assert_eq!(batch.scanned_tasks, 1);
        assert_eq!(batch.skipped_tasks, 1);
    }

    #[test]
    fn falls_back_to_proc_stat() {
        let dir = tempfile::tempdir().unwrap();
        let task = task_info(101, 100);
        let stat_a = "101 (Game Thread) S 1 1 1 0 -1 0 0 0 0 0 10 5 0 0 20 0 1 0 0";
        let stat_b = "101 (Game Thread) S 1 1 1 0 -1 0 0 0 0 0 20 10 0 0 20 0 1 0 0";
        write_task_file(dir.path(), 100, 101, "stat", stat_a);

        let mut sampler = RuntimeSliceSampler::with_proc_root(dir.path().to_path_buf());
        sampler.clock_ticks_per_second = 100;
        let first = sampler.collect(std::slice::from_ref(&task), 1000, 1000, 256);
        assert!(first.records.is_empty());

        write_task_file(dir.path(), 100, 101, "stat", stat_b);
        let second = sampler.collect(std::slice::from_ref(&task), 2000, 1000, 256);
        assert_eq!(second.records.len(), 1);
        let record = &second.records[0];
        assert_eq!(record.source, RuntimeSliceSource::ProcStatFallback);
        assert_eq!(record.user_runtime_delta_ns, Some(100_000_000));
        assert_eq!(record.system_runtime_delta_ns, Some(50_000_000));
        assert_eq!(record.runtime_delta_ns, 150_000_000);
        assert_eq!(record.runqueue_wait_delta_ns, None);
    }

    #[test]
    fn rejects_malformed_proc_stat() {
        assert!(parse_proc_stat_runtime_ticks("101 Game Thread S").is_err());
    }

    #[test]
    fn handles_task_disappearance() {
        let dir = tempfile::tempdir().unwrap();
        let task = task_info(101, 100);
        write_task_file(dir.path(), 100, 101, "schedstat", "1000 2000 4\n");

        let mut sampler = RuntimeSliceSampler::with_proc_root(dir.path().to_path_buf());
        sampler.collect(std::slice::from_ref(&task), 1000, 1000, 256);
        fs::remove_dir_all(dir.path().join("100")).unwrap();
        let batch = sampler.collect(std::slice::from_ref(&task), 2000, 1000, 256);

        assert!(batch.records.is_empty());
        assert_eq!(batch.read_errors, 1);
    }
}
