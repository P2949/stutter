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
