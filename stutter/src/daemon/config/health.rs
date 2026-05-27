use serde::{Deserialize, Serialize};

use crate::daemon::health::SystemHealthThresholds;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DaemonHealthConfig {
    pub max_cpu_temp_celsius: u32,
    pub max_gpu_temp_celsius: u32,
    pub min_disk_available_bytes: u64,
    pub max_memory_pressure_some_avg10_percent: f32,
}

impl Default for DaemonHealthConfig {
    fn default() -> Self {
        let thresholds = SystemHealthThresholds::default();

        Self {
            max_cpu_temp_celsius: (thresholds.max_cpu_temp_millidegrees / 1000) as u32,
            max_gpu_temp_celsius: (thresholds.max_gpu_temp_millidegrees / 1000) as u32,
            min_disk_available_bytes: thresholds.min_disk_available_bytes,
            max_memory_pressure_some_avg10_percent: thresholds
                .max_memory_pressure_some_avg10_millipercent
                as f32
                / 1000.0,
        }
    }
}

impl DaemonHealthConfig {
    pub fn thresholds(&self) -> SystemHealthThresholds {
        let defaults = SystemHealthThresholds::default();

        SystemHealthThresholds {
            max_cpu_temp_millidegrees: i64::from(self.max_cpu_temp_celsius) * 1000,
            max_gpu_temp_millidegrees: i64::from(self.max_gpu_temp_celsius) * 1000,
            min_disk_available_bytes: self.min_disk_available_bytes,
            max_memory_pressure_some_avg10_millipercent: (self
                .max_memory_pressure_some_avg10_percent
                * 1000.0)
                .round() as u32,
            max_load_per_cpu_milli: defaults.max_load_per_cpu_milli,
            max_ebpf_dropped_events: defaults.max_ebpf_dropped_events,
        }
    }
}
