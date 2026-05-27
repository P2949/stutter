use std::{collections::BTreeMap, path::PathBuf};

use serde::Serialize;

#[derive(Debug, Clone)]
pub struct DoctorInput {
    pub json: bool,
    pub tracepoint_dump: bool,
    pub hwmon: bool,
    pub hwmon_root: Option<PathBuf>,
    pub hwmon_drm_card: Option<String>,
    pub hwmon_render_node: Option<PathBuf>,
    pub irq_latency: bool,
    pub irqs: Vec<u32>,
    pub block_io: bool,
    pub kms_timing: bool,
    pub faults: bool,
    pub cpu_perf: bool,
    pub mangohud_log: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub enum DoctorStatus {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Serialize)]
pub struct DoctorReport {
    pub overall: DoctorStatus,
    pub checks: Vec<DoctorCheck>,
}

#[derive(Debug, Serialize)]
pub struct DoctorCheck {
    pub name: String,
    pub status: DoctorStatus,
    pub message: String,
    pub details: BTreeMap<String, String>,
}
