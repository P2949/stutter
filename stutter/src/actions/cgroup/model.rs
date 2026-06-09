//! Cgroup placement action data model.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{actions::TaskIdentity, process_tree::TaskClass};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CgroupPlacementPolicy {
    pub allow_cgroup_moves: bool,
    pub allow_cpuset_changes: bool,
    pub allow_nested_cgroups: bool,
}

impl Default for CgroupPlacementPolicy {
    fn default() -> Self {
        Self {
            allow_cgroup_moves: true,
            allow_cpuset_changes: true,
            allow_nested_cgroups: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CgroupPlacementTarget {
    pub identity: TaskIdentity,
    pub class: TaskClass,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CgroupPlacementAction {
    pub cgroup_root: PathBuf,
    pub target_cgroup: PathBuf,
    pub targets: Vec<CgroupPlacementTarget>,
    pub cpuset_cpus: Option<String>,
    pub cpuset_mems: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CgroupTargetSnapshot {
    pub(super) tid: u32,
    pub(super) process_pid: Option<u32>,
    pub(super) comm: Option<String>,
    pub(super) starttime_ticks: Option<u64>,
    pub(super) exe: Option<PathBuf>,
    pub(super) original_cgroup: PathBuf,
}
