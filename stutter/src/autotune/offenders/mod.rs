//! Sustained background CPU consumers that compete with a focused workload.
//!
//! A scheduler-pressure candidate that lowers the priority of competing work needs to know *who*
//! competes. The sampler reads cumulative CPU time from procfs on every observation, derives each
//! process's CPU share over the elapsed interval, and reports a process only after it stayed above
//! the threshold for several consecutive samples. It never guesses an identity: every reported
//! process carries its start time, owner and per-task identities, and anything it cannot read
//! completely is left out.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    os::unix::fs::MetadataExt,
    path::Path,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use stutter_core::ids::Pid;

use crate::{
    actions::TaskIdentity,
    autotune::observation::is_protected_task_class,
    process_tree::{TaskClass, classify_task},
};

/// A process must use at least this share of one CPU...
pub const OFFENDER_MIN_CPU_PERCENT: f64 = 40.0;
/// ...for this many consecutive samples before it is an offender.
pub const OFFENDER_MIN_SUSTAINED_SAMPLES: u32 = 3;
/// At most this many offenders are ever reported.
pub const OFFENDER_MAX_REPORTED: usize = 4;
/// Samples closer together than this are too noisy to derive a rate from.
const MIN_SAMPLE_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OffenderProcess {
    pub pid: Pid,
    pub starttime_ticks: u64,
    pub comm: String,
    pub uid: u32,
    pub nice: i32,
    /// Share of one CPU over the last sample interval.
    pub cpu_percent: f64,
    pub sustained_samples: u32,
    /// Every task of the process with its own start time, read at sampling time.
    pub tasks: Vec<TaskIdentity>,
    /// The niceness each task had when it was sampled (the active configuration the apply
    /// preflight compares against).
    #[serde(default)]
    pub task_nice: BTreeMap<u32, i32>,
}

#[derive(Clone, Debug)]
struct Seen {
    starttime_ticks: u64,
    cpu_ticks: u64,
    hot_samples: u32,
}

#[derive(Debug, Default)]
pub struct OffenderSampler {
    previous: BTreeMap<u32, Seen>,
    last_sample: Option<std::time::Instant>,
}

struct StatFields {
    comm: String,
    flags: u64,
    cpu_ticks: u64,
    nice: i32,
    starttime_ticks: u64,
}

fn parse_stat(text: &str) -> Option<StatFields> {
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    let comm = text.get(open + 1..close)?.to_owned();
    let tail: Vec<&str> = text.get(close + 1..)?.split_whitespace().collect();
    let field = |index: usize| tail.get(index)?.parse::<i64>().ok();
    Some(StatFields {
        comm,
        flags: u64::try_from(field(6)?).ok()?,
        cpu_ticks: u64::try_from(field(11)? + field(12)?).ok()?,
        nice: i32::try_from(field(16)?).ok()?,
        starttime_ticks: u64::try_from(field(19)?).ok()?,
    })
}

const PF_KTHREAD: u64 = 0x0020_0000;

impl OffenderSampler {
    /// Takes one sample and returns the processes that have been above the threshold for long
    /// enough. `exclude` lists processes that must never be reported (the focused workload).
    pub fn sample(
        &mut self,
        proc_root: &Path,
        clock_ticks_per_second: u64,
        owner_uid: u32,
        exclude: &BTreeSet<u32>,
        only_descendants_of: Option<u32>,
    ) -> Vec<OffenderProcess> {
        self.sample_at(
            std::time::Instant::now(),
            proc_root,
            clock_ticks_per_second,
            owner_uid,
            exclude,
            only_descendants_of,
        )
    }

    fn sample_at(
        &mut self,
        now: std::time::Instant,
        proc_root: &Path,
        clock_ticks_per_second: u64,
        owner_uid: u32,
        exclude: &BTreeSet<u32>,
        only_descendants_of: Option<u32>,
    ) -> Vec<OffenderProcess> {
        let elapsed = self.last_sample.map(|at| now.duration_since(at));
        let usable_interval = elapsed.filter(|elapsed| *elapsed >= MIN_SAMPLE_INTERVAL);
        let mut current = BTreeMap::new();
        let mut offenders = Vec::new();
        let Ok(entries) = fs::read_dir(proc_root) else {
            return Vec::new();
        };
        for entry in entries.flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|n| n.parse::<u32>().ok())
            else {
                continue;
            };
            if pid <= 1 || exclude.contains(&pid) {
                continue;
            }
            let Ok(text) = fs::read_to_string(entry.path().join("stat")) else {
                continue;
            };
            let Some(stat) = parse_stat(&text) else {
                continue;
            };
            if stat.flags & PF_KTHREAD != 0 {
                continue;
            }
            let hot_samples = match (self.previous.get(&pid), usable_interval) {
                (Some(before), Some(elapsed)) if before.starttime_ticks == stat.starttime_ticks => {
                    let seconds = elapsed.as_secs_f64();
                    let used = stat.cpu_ticks.saturating_sub(before.cpu_ticks) as f64
                        / clock_ticks_per_second.max(1) as f64;
                    let percent = used / seconds * 100.0;
                    if percent >= OFFENDER_MIN_CPU_PERCENT {
                        let hot = before.hot_samples.saturating_add(1);
                        if hot >= OFFENDER_MIN_SUSTAINED_SAMPLES
                            && only_descendants_of
                                .is_none_or(|ancestor| descends_from(proc_root, pid, ancestor))
                            && let Some(process) =
                                describe(proc_root, pid, &stat, percent, hot, owner_uid)
                        {
                            offenders.push(process);
                        }
                        hot
                    } else {
                        0
                    }
                }
                // Not enough history to judge: carry the count over unchanged.
                (Some(before), None) if before.starttime_ticks == stat.starttime_ticks => {
                    before.hot_samples
                }
                _ => 0,
            };
            current.insert(
                pid,
                Seen {
                    starttime_ticks: stat.starttime_ticks,
                    cpu_ticks: stat.cpu_ticks,
                    hot_samples,
                },
            );
        }
        // A too-close sample must not move the reference time, or the next rate would be skewed.
        if usable_interval.is_some() || self.last_sample.is_none() {
            self.last_sample = Some(now);
            self.previous = current;
        }
        offenders.sort_by(|a, b| b.cpu_percent.total_cmp(&a.cpu_percent));
        offenders.truncate(OFFENDER_MAX_REPORTED);
        offenders
    }
}

/// Builds the full description of a hot process, or `None` when it is not a safe offender: another
/// user's, protected infrastructure, a game, or one whose tasks cannot all be identified.
fn describe(
    proc_root: &Path,
    pid: u32,
    stat: &StatFields,
    cpu_percent: f64,
    sustained_samples: u32,
    owner_uid: u32,
) -> Option<OffenderProcess> {
    let process_dir = proc_root.join(pid.to_string());
    let uid = fs::metadata(&process_dir).ok()?.uid();
    if uid != owner_uid {
        return None;
    }
    let cmdline = fs::read(process_dir.join("cmdline"))
        .map(|bytes| String::from_utf8_lossy(&bytes).replace('\0', " "))
        .unwrap_or_default();
    let class = classify_task(&stat.comm, &stat.comm, &cmdline);
    if is_protected_task_class(class)
        || matches!(
            class,
            TaskClass::Game
                | TaskClass::GameRenderThread
                | TaskClass::GameWorkerThread
                | TaskClass::GameHelper
                | TaskClass::WineServer
        )
    {
        return None;
    }
    let mut tasks = Vec::new();
    let mut task_nice = BTreeMap::new();
    for entry in fs::read_dir(process_dir.join("task")).ok()?.flatten() {
        let tid = entry.file_name().to_str()?.parse::<u32>().ok()?;
        let task = parse_stat(&fs::read_to_string(entry.path().join("stat")).ok()?)?;
        task_nice.insert(tid, task.nice);
        tasks.push(TaskIdentity {
            tid: tid.into(),
            process_pid: Some(pid.into()),
            comm: Some(task.comm),
            starttime_ticks: Some(task.starttime_ticks),
        });
    }
    if tasks.is_empty() {
        return None;
    }
    Some(OffenderProcess {
        pid: pid.into(),
        starttime_ticks: stat.starttime_ticks,
        comm: stat.comm.clone(),
        uid,
        nice: stat.nice,
        cpu_percent,
        sustained_samples,
        tasks,
        task_nice,
    })
}

/// Whether `ancestor` is on the parent chain of `pid` (a validation run only ever considers the
/// contention its own harness started, never the user's real work).
fn descends_from(proc_root: &Path, pid: u32, ancestor: u32) -> bool {
    let mut current = pid;
    for _ in 0..64 {
        let Some(parent) = fs::read_to_string(proc_root.join(current.to_string()).join("stat"))
            .ok()
            .and_then(|stat| {
                stat.rsplit_once(')')?
                    .1
                    .split_whitespace()
                    .nth(1)?
                    .parse::<u32>()
                    .ok()
            })
        else {
            return false;
        };
        if parent == ancestor {
            return true;
        }
        if parent <= 1 {
            return false;
        }
        current = parent;
    }
    false
}

#[cfg(test)]
mod tests;
