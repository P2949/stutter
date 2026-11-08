//! The runtime's sensor registry: cheap, always-on, read-only.
//!
//! Only the system PSI sensor is activated; expensive sensors need a logged diagnostic hypothesis
//! and are not started here. Readings ride on the observation for status and diagnosis and never
//! trigger a write on their own. Each sensor's cumulative sampling cost is kept so the daemon's
//! incremental overhead stays visible.
use std::{path::Path, time::Instant};

use crate::autotune::sensors::{
    AdmissionDenied, CpuPowerSensor, ExpensiveBudget, OomGuardSensor, PsiSensor, SensorReading,
    SensorRegistry, session_cgroup_dir,
};

pub(super) struct SensorHub {
    registry: SensorRegistry,
    started: Instant,
    /// Why a sensor could not be admitted, kept for diagnostics instead of failing the runtime.
    unavailable: Vec<(&'static str, String)>,
}

impl SensorHub {
    pub(super) fn new(proc_root: &Path) -> Self {
        let mut registry = SensorRegistry::new(ExpensiveBudget::default());
        registry.register(Box::new(PsiSensor::system(proc_root)));
        registry.register(Box::new(CpuPowerSensor::new(
            Path::new("/sys/devices/system"),
            Path::new("/sys/class"),
        )));
        let started = Instant::now();
        let mut unavailable = Vec::new();
        let mut ids = vec!["psi_system", "cpu_power"];
        let session = std::fs::read_to_string(proc_root.join("self/cgroup"))
            .ok()
            .and_then(|text| {
                session_cgroup_dir(
                    Path::new("/sys/fs/cgroup"),
                    &text,
                    crate::syscall::geteuid(),
                )
            });
        if let Some(dir) = session {
            registry.register(Box::new(OomGuardSensor::new(dir)));
            ids.push("oom_guard");
        }
        for id in ids {
            if let Err(denied) = registry.activate(id, 0) {
                log::info!("sensor {id} unavailable: {}", describe(&denied));
                unavailable.push((id, describe(&denied)));
            }
        }
        Self {
            registry,
            started,
            unavailable,
        }
    }

    /// Samples every running sensor now.
    pub(super) fn sample(&mut self) -> Vec<SensorReading> {
        let now_ns = self.started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
        self.registry.sample_all(now_ns)
    }

    #[cfg(test)]
    pub(super) fn unavailable(&self) -> &[(&'static str, String)] {
        &self.unavailable
    }
}

fn describe(denied: &AdmissionDenied) -> String {
    match denied {
        AdmissionDenied::Unavailable(reason) | AdmissionDenied::StartFailed(reason) => {
            reason.clone()
        }
        other => format!("{other:?}"),
    }
}

impl std::fmt::Debug for SensorHub {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SensorHub")
            .field("unavailable", &self.unavailable)
            .finish()
    }
}
