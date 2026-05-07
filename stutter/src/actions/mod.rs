#![allow(dead_code)]

pub mod cpu_affinity;
pub mod runner;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum SafetyClass {
    #[default]
    ObserveOnly,
    ReversibleLowRisk,
    ReversibleMediumRisk,
    HighRisk,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionId(pub String);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionWarning {
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionState {
    pub applied: bool,
    pub affected_tasks: usize,
    pub checked_tasks: usize,
    pub pending_changes: usize,
    pub warnings: Vec<ActionWarning>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NiceRestoreRecord {
    pub tid: u32,
    pub original_nice: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IoPrioRestoreRecord {
    pub tid: u32,
    pub original_ioprio: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CgroupRestoreRecord {
    pub pid: u32,
    pub original_cgroup: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionOutcome {
    pub action_id: ActionId,
    pub safety_class: SafetyClass,
    pub dry_run: bool,
    pub preflight_warnings: Vec<ActionWarning>,
    pub state: ActionState,
    pub rollback: Option<RollbackToken>,
    pub started_unix_nanos: u128,
    pub finished_unix_nanos: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind")]
pub enum RollbackToken {
    #[serde(rename = "cpu-affinity-restore-file")]
    CpuAffinityRestoreFile {
        #[serde(rename = "restore_path")]
        path: PathBuf,
        affected_tasks: usize,
    },
    NiceRestore {
        records: Vec<NiceRestoreRecord>,
    },
    IoPrioRestore {
        records: Vec<IoPrioRestoreRecord>,
    },
    CgroupRestore {
        records: Vec<CgroupRestoreRecord>,
    },
    SysfsRestore {
        path: PathBuf,
        original_value: String,
    },
}

impl RollbackToken {
    pub fn affected_tasks(&self) -> usize {
        match self {
            Self::CpuAffinityRestoreFile { affected_tasks, .. } => *affected_tasks,
            Self::NiceRestore { records } => records.len(),
            Self::IoPrioRestore { records } => records.len(),
            Self::CgroupRestore { records } => records.len(),
            Self::SysfsRestore { .. } => 1,
        }
    }

    pub fn restore_path(&self) -> Option<&PathBuf> {
        match self {
            Self::CpuAffinityRestoreFile { path, .. } => Some(path),
            Self::SysfsRestore { path, .. } => Some(path),
            Self::NiceRestore { .. } | Self::IoPrioRestore { .. } | Self::CgroupRestore { .. } => {
                None
            }
        }
    }
}

pub trait TuningAction {
    fn id(&self) -> ActionId;
    fn describe(&self) -> String;
    fn safety_class(&self) -> SafetyClass;
    fn preflight(&self) -> anyhow::Result<Vec<ActionWarning>>;
    fn dry_run(&self) -> anyhow::Result<ActionState>;
    fn apply(&self) -> anyhow::Result<RollbackToken>;
    fn verify(&self) -> anyhow::Result<ActionState>;
    fn rollback(&self, token: &RollbackToken) -> anyhow::Result<()>;
}
