use serde::{Deserialize, Serialize};
use stutter_core::ids::{Pid, Tid};

use crate::affinity::AffinityRecord;

pub const PROFILE_RESTORE_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProfileRestoreState {
    pub schema_version: u32,
    #[serde(default)]
    pub affinity_records: Vec<AffinityRecord>,
    #[serde(default)]
    pub nice_records: Vec<NiceRestoreRecordV2>,
    #[serde(default)]
    pub ionice_records: Vec<IoPrioRestoreRecordV2>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NiceRestoreRecordV2 {
    pub tid: Tid,
    #[serde(default)]
    pub process_pid: Option<Pid>,
    #[serde(default)]
    pub process_starttime_ticks: Option<u64>,
    #[serde(default)]
    pub task_starttime_ticks: Option<u64>,
    #[serde(default)]
    pub comm: Option<String>,
    pub original_nice: i32,
    pub applied_nice: i32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IoPrioRestoreRecordV2 {
    pub tid: Tid,
    #[serde(default)]
    pub process_pid: Option<Pid>,
    #[serde(default)]
    pub process_starttime_ticks: Option<u64>,
    #[serde(default)]
    pub task_starttime_ticks: Option<u64>,
    #[serde(default)]
    pub comm: Option<String>,
    pub original_ioprio: i32,
    pub applied_ioprio: i32,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProfileRestoreSummary {
    pub affinity: usize,
    pub nice: usize,
    pub ionice: usize,
    pub skipped_dead: usize,
    pub skipped_identity_mismatch: usize,
    pub legacy_unverified: usize,
    pub errors: usize,
}

impl ProfileRestoreSummary {
    pub fn restored_total(&self) -> usize {
        self.affinity + self.nice + self.ionice
    }
}

#[derive(Clone, Copy)]
pub(crate) struct RestoreIdentity {
    pub process_pid: Option<Pid>,
    pub process_starttime_ticks: Option<u64>,
    pub task_starttime_ticks: Option<u64>,
}

impl NiceRestoreRecordV2 {
    pub(crate) fn identity_tuple(&self) -> RestoreIdentity {
        RestoreIdentity {
            process_pid: self.process_pid,
            process_starttime_ticks: self.process_starttime_ticks,
            task_starttime_ticks: self.task_starttime_ticks,
        }
    }
}

impl IoPrioRestoreRecordV2 {
    pub(crate) fn identity_tuple(&self) -> RestoreIdentity {
        RestoreIdentity {
            process_pid: self.process_pid,
            process_starttime_ticks: self.process_starttime_ticks,
            task_starttime_ticks: self.task_starttime_ticks,
        }
    }
}
