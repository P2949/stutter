use serde::{Deserialize, Serialize};

use crate::actions::TaskIdentity;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum IoPrioClass {
    None,
    Realtime,
    BestEffort,
    Idle,
}

impl IoPrioClass {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Realtime => "realtime",
            Self::BestEffort => "best-effort",
            Self::Idle => "idle",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct IoPrioValue {
    pub class: IoPrioClass,
    pub level: Option<u8>,
}

impl IoPrioValue {
    pub fn best_effort(level: u8) -> Self {
        Self {
            class: IoPrioClass::BestEffort,
            level: Some(level),
        }
    }

    pub fn realtime(level: u8) -> Self {
        Self {
            class: IoPrioClass::Realtime,
            level: Some(level),
        }
    }

    pub fn idle() -> Self {
        Self {
            class: IoPrioClass::Idle,
            level: None,
        }
    }

    pub fn none() -> Self {
        Self {
            class: IoPrioClass::None,
            level: None,
        }
    }

    pub fn label(self) -> String {
        match self.level {
            Some(level) => format!("{}:{level}", self.class.label()),
            None => self.class.label().to_owned(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IoPrioPolicy {
    pub allow_ioprio_changes: bool,
    pub allow_realtime_class: bool,
    pub allow_none_class: bool,
    pub max_best_effort_level: u8,
    pub require_strong_block_io_evidence: bool,
    pub strong_block_io_evidence: bool,
}

impl Default for IoPrioPolicy {
    fn default() -> Self {
        Self {
            allow_ioprio_changes: false,
            allow_realtime_class: false,
            allow_none_class: false,
            max_best_effort_level: 7,
            require_strong_block_io_evidence: true,
            strong_block_io_evidence: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IoPrioAction {
    pub targets: Vec<TaskIdentity>,
    pub ioprio: IoPrioValue,
    pub policy: IoPrioPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IoPrioTargetSnapshot {
    pub(crate) tid: u32,
    pub(crate) process_pid: Option<u32>,
    pub(crate) comm: Option<String>,
    pub(crate) starttime_ticks: Option<u64>,
    pub(crate) exe: Option<std::path::PathBuf>,
    pub(crate) current_ioprio: i32,
    pub(crate) current_value: IoPrioValue,
}
