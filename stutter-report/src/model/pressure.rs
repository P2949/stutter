use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PressureTimelineSummary {
    pub sample_count: usize,
    pub max_cpu_some: f64,
    pub max_mem_some: Option<f64>,
    pub max_mem_full: Option<f64>,
    pub max_io_some: Option<f64>,
    pub max_io_full: Option<f64>,
    pub windows: Vec<PressureWindow>,
    pub peak_windows: Vec<PressurePeakWindow>,
    pub pressure_notes: Vec<String>,
    pub coverage: PressureTimelineCoverage,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PressureTimelineCoverage {
    pub interval_records_loaded: usize,
    pub has_cpu_psi: bool,
    pub has_mem_psi: bool,
    pub has_io_psi: bool,
    pub has_near_spike_windows: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PressurePeakWindow {
    pub elapsed_ms: u64,
    pub pressure_kind: PressureKind,
    pub value: f64,
    pub near_spike: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PressureKind {
    CpuSome,
    MemSome,
    MemFull,
    IoSome,
    IoFull,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PressureWindow {
    pub elapsed_ms: u64,
    pub cpu_some: f64,
    pub mem_some: Option<f64>,
    pub mem_full: Option<f64>,
    pub io_some: Option<f64>,
    pub io_full: Option<f64>,
    pub near_spike: bool,
}
