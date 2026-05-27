use serde::{Deserialize, Serialize};

use crate::actions::TaskIdentity;

pub const UCLAMP_MIN_VALUE: u32 = 0;
pub const UCLAMP_MAX_VALUE: u32 = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct UclampValues {
    pub sched_util_min: Option<u32>,
    pub sched_util_max: Option<u32>,
}

impl UclampValues {
    pub fn is_empty(self) -> bool {
        self.sched_util_min.is_none() && self.sched_util_max.is_none()
    }

    pub(super) fn requested_min_or(self, current: UclampCurrentValues) -> u32 {
        self.sched_util_min.unwrap_or(current.sched_util_min)
    }

    pub(super) fn requested_max_or(self, current: UclampCurrentValues) -> u32 {
        self.sched_util_max.unwrap_or(current.sched_util_max)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UclampCurrentValues {
    pub sched_util_min: u32,
    pub sched_util_max: u32,
}

#[derive(Debug, Clone)]
pub struct UclampPolicy {
    pub allow_uclamp_changes: bool,
    pub min_allowed_util_min: u32,
    pub max_allowed_util_min: u32,
    pub min_allowed_util_max: u32,
    pub max_allowed_util_max: u32,
    pub allow_per_task: bool,
    pub allow_cgroup: bool,
}

impl Default for UclampPolicy {
    fn default() -> Self {
        Self {
            allow_uclamp_changes: true,
            min_allowed_util_min: UCLAMP_MIN_VALUE,
            max_allowed_util_min: UCLAMP_MAX_VALUE,
            min_allowed_util_max: UCLAMP_MIN_VALUE,
            max_allowed_util_max: UCLAMP_MAX_VALUE,
            allow_per_task: true,
            allow_cgroup: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UclampAction {
    pub targets: Vec<TaskIdentity>,
    pub values: UclampValues,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UclampTargetSnapshot {
    pub(crate) tid: u32,
    pub(crate) process_pid: Option<u32>,
    pub(crate) comm: Option<String>,
    pub(crate) starttime_ticks: Option<u64>,
    pub(crate) exe: Option<std::path::PathBuf>,
    pub(crate) current: UclampCurrentValues,
}
