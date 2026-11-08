//! What the user (or a launcher) has already decided for a task, read back from the kernel.
//!
//! A restricted CPU mask, a non-default nice value or a real-time policy on a task is somebody's
//! intent. Affinity and nice become `UserExplicit` receipts (ownership constraints, never
//! overridden by inference). A real-time policy is reported for diagnosis only: Stutter never
//! promotes a task to FIFO, RR or DEADLINE and has no code path that could.
use std::{fs, path::Path};

use serde::{Deserialize, Serialize};

use super::receipt::{OwnerClass, ResourceKey, ResourceReceipt, TaskAttribute};
use crate::topology::{parse_cpu_list, sorted_unique};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SchedPolicy {
    Normal,
    Fifo,
    RoundRobin,
    Batch,
    Idle,
    Deadline,
    Other(u32),
}

impl SchedPolicy {
    pub fn from_raw(raw: u32) -> Self {
        match raw {
            0 => Self::Normal,
            1 => Self::Fifo,
            2 => Self::RoundRobin,
            3 => Self::Batch,
            5 => Self::Idle,
            6 => Self::Deadline,
            other => Self::Other(other),
        }
    }

    pub fn is_real_time(self) -> bool {
        matches!(self, Self::Fifo | Self::RoundRobin | Self::Deadline)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskIntent {
    pub cpus_allowed: Vec<u32>,
    pub nice: i32,
    pub policy: SchedPolicy,
}

/// The intent of `pid`, from its `stat` and `status`. `None` when it cannot be read.
pub fn read_task_intent(proc_root: &Path, pid: u32) -> Option<TaskIntent> {
    let dir = proc_root.join(pid.to_string());
    let stat = fs::read_to_string(dir.join("stat")).ok()?;
    let (_, after_comm) = stat.rsplit_once(") ")?;
    let fields: Vec<&str> = after_comm.split_whitespace().collect();
    let nice: i32 = fields.get(16)?.parse().ok()?;
    let policy = SchedPolicy::from_raw(fields.get(38)?.parse().ok()?);
    let status = fs::read_to_string(dir.join("status")).ok()?;
    let cpus_allowed = status
        .lines()
        .find_map(|line| line.strip_prefix("Cpus_allowed_list:"))
        .and_then(|list| parse_cpu_list(list.trim()).ok())
        .map(sorted_unique)?;
    Some(TaskIntent {
        cpus_allowed,
        nice,
        policy,
    })
}

/// What the task's settings say about who decided them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntentReport {
    pub affinity_restricted: bool,
    pub nice_set: bool,
    /// Diagnostic only; Stutter never writes a real-time policy.
    pub real_time: bool,
}

pub fn classify(intent: &TaskIntent, online: &[u32]) -> IntentReport {
    let online = sorted_unique(online.to_vec());
    IntentReport {
        affinity_restricted: !intent.cpus_allowed.is_empty() && intent.cpus_allowed != online,
        nice_set: intent.nice != 0,
        real_time: intent.policy.is_real_time(),
    }
}

/// `UserExplicit` receipts for the attributes of one task that already carry someone's decision.
pub fn user_receipts(
    pid: u32,
    start_ticks: u64,
    intent: &TaskIntent,
    online: &[u32],
    now_ns: u64,
    ttl_ns: u64,
) -> Vec<ResourceReceipt> {
    let report = classify(intent, online);
    let receipt = |attribute: TaskAttribute, value: String| ResourceReceipt {
        key: ResourceKey::Task {
            pid,
            start_ticks,
            attribute,
        },
        owner: OwnerClass::UserExplicit,
        source: "kernel task state (affinity, nice)".to_owned(),
        observed_at_ns: now_ns,
        expires_at_ns: now_ns.saturating_add(ttl_ns),
        observed_value: value,
    };
    let mut receipts = Vec::new();
    if report.affinity_restricted {
        receipts.push(receipt(
            TaskAttribute::Affinity,
            format!("{:?}", intent.cpus_allowed),
        ));
    }
    if report.nice_set {
        receipts.push(receipt(TaskAttribute::Nice, intent.nice.to_string()));
    }
    receipts
}

#[cfg(test)]
#[path = "user_intent_tests.rs"]
mod tests;

/// The first of `targets` whose nice value is not the default: someone set it, and a Nice
/// candidate must not override that decision.
pub fn explicit_nice_conflict(
    nice_per_tid: &std::collections::BTreeMap<u32, i32>,
    targets: &[u32],
) -> Option<u32> {
    targets
        .iter()
        .copied()
        .find(|tid| nice_per_tid.get(tid).is_some_and(|nice| *nice != 0))
}

/// The first of `targets` with an explicit I/O priority. The labels are those of
/// `IoPrioValue::label`. `none` and the kernel's default best-effort level 4 are nobody's decision;
/// a real-time or idle class, or any other best-effort level, is.
pub fn explicit_ioprio_conflict(
    ioprio_per_tid: &std::collections::BTreeMap<u32, String>,
    targets: &[u32],
) -> Option<u32> {
    targets.iter().copied().find(|tid| {
        ioprio_per_tid.get(tid).is_some_and(|label| {
            let label = label.trim();
            !label.is_empty() && !label.starts_with("none") && label != "best-effort:4"
        })
    })
}

/// The first of `targets` with a utilization clamp other than the kernel default (0 and 1024).
pub fn explicit_uclamp_conflict(
    uclamp_per_tid: &std::collections::BTreeMap<u32, crate::actions::uclamp::UclampValues>,
    targets: &[u32],
) -> Option<u32> {
    targets.iter().copied().find(|tid| {
        uclamp_per_tid.get(tid).is_some_and(|values| {
            values.sched_util_min.is_some_and(|min| min > 0)
                || values.sched_util_max.is_some_and(|max| max < 1024)
        })
    })
}

/// Whether `numa_maps` shows a NUMA memory policy someone set. Every mapping line carries its
/// policy as the word after the address; `default` is nobody's decision. Bounded to the first
/// 4096 lines of a possibly huge file.
pub fn mempolicy_is_explicit(numa_maps: &str) -> bool {
    numa_maps.lines().take(4096).any(|line| {
        line.split_whitespace()
            .nth(1)
            .is_some_and(|policy| policy != "default")
    })
}

/// Whether `pid` has an explicit NUMA memory policy, from its `numa_maps`. `None` when the file is
/// unreadable or absent (no NUMA, no permission): unknown is not "default".
pub fn read_mempolicy_explicit(proc_root: &Path, pid: u32) -> Option<bool> {
    let file = fs::File::open(proc_root.join(pid.to_string()).join("numa_maps")).ok()?;
    let mut text = String::new();
    // A bounded read: the file can be very large and the first lines carry the policy.
    std::io::Read::read_to_string(&mut std::io::Read::take(file, 512 * 1024), &mut text).ok()?;
    Some(mempolicy_is_explicit(&text))
}

/// What a systemd unit or an administrator already decided for a cgroup, from its own files:
/// a non-default `cpu.weight` (100), a `cpu.max` quota, or `cpu.idle`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CgroupIntent {
    pub cpu_weight_set: bool,
    pub cpu_max_set: bool,
    pub cpu_idle_set: bool,
}

pub fn read_cgroup_intent(cgroup_dir: &Path) -> CgroupIntent {
    let read = |name: &str| {
        fs::read_to_string(cgroup_dir.join(name))
            .ok()
            .map(|t| t.trim().to_owned())
    };
    CgroupIntent {
        cpu_weight_set: read("cpu.weight").is_some_and(|weight| weight != "100"),
        cpu_max_set: read("cpu.max").is_some_and(|max| !max.starts_with("max")),
        cpu_idle_set: read("cpu.idle").is_some_and(|idle| idle == "1"),
    }
}
