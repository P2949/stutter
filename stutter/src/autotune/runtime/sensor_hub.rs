//! The runtime's sensor registry: cheap, always-on, read-only.
//!
//! The cheap sensors start at once. The PMU sensor is registered but started only when the escalator
//! sees sustained pressure and logs the hypothesis; the registry then admits it under its budget. Readings ride on the observation for status and diagnosis and never
//! trigger a write on their own. Each sensor's cumulative sampling cost is kept so the daemon's
//! incremental overhead stays visible.
use std::{path::Path, time::Instant};

use crate::{
    autotune::sensors::{
        AdmissionDenied, BlockIoSensor, CpuIdleSensor, CpuPowerSensor, DamonSensor, Escalator,
        ExpensiveBudget, NetworkSensor, NumaFaultSensor, NumaSensor, OomGuardSensor, PmuSensor,
        PmuTargets, PsiSensor, ResctrlSensor, SchedExtSensor, SensorReading, SensorRegistry,
        Thresholds, session_cgroup_dir,
    },
    perf_counters::CpuPerfConfig,
};

pub(super) struct SensorHub {
    registry: SensorRegistry,
    cgroup_root: std::path::PathBuf,
    /// The cgroup whose pressure the `psi_cgroup` sensor currently follows.
    tracked_cgroup: Option<String>,
    escalator: Escalator,
    pmu_targets: PmuTargets,
    started: Instant,
    /// Why a sensor could not be admitted, kept for diagnostics instead of failing the runtime.
    unavailable: Vec<(&'static str, String)>,
}

impl SensorHub {
    pub(super) fn new(proc_root: &Path) -> Self {
        Self::with_cgroup_root(proc_root, Path::new("/sys/fs/cgroup"))
    }

    pub(super) fn with_cgroup_root(proc_root: &Path, cgroup_root: &Path) -> Self {
        let mut registry = SensorRegistry::new(ExpensiveBudget::default());
        // The PMU sensor is registered but never activated here: only an escalation starts it.
        let pmu = PmuSensor::new(CpuPerfConfig {
            include_kernel: false,
            max_tasks: 8,
            collect_cache_refs: true,
        });
        let pmu_targets = pmu.targets_handle();
        registry.register(Box::new(pmu));
        registry.register(Box::new(PsiSensor::system(proc_root)));
        registry.register(Box::new(CpuPowerSensor::new(
            Path::new("/sys/devices/system"),
            Path::new("/sys/class"),
        )));
        let started = Instant::now();
        let mut unavailable = Vec::new();
        registry.register(Box::new(NumaSensor::new(Path::new("/sys/devices/system"))));
        registry.register(Box::new(DamonSensor::new(Path::new("/sys/kernel/mm"))));
        registry.register(Box::new(ResctrlSensor::new(Path::new("/sys/fs"))));
        registry.register(Box::new(NetworkSensor::new(proc_root.to_path_buf())));
        registry.register(Box::new(NumaFaultSensor::new(proc_root.to_path_buf())));
        registry.register(Box::new(CpuIdleSensor::new(
            Path::new("/sys/devices/system"),
            Path::new("/sys/class"),
        )));
        registry.register(Box::new(SchedExtSensor::new(std::path::PathBuf::from(
            "/sys",
        ))));
        registry.register(Box::new(BlockIoSensor::new(
            proc_root.to_path_buf(),
            std::path::PathBuf::from("/sys"),
        )));
        let mut ids = vec![
            "psi_system",
            "cpu_power",
            "numa_stat",
            "damon_state",
            "resctrl_inventory",
            "network",
            "numa_faults",
            "cpu_idle",
            "sched_ext",
            "block_io",
        ];
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
            cgroup_root: cgroup_root.to_path_buf(),
            tracked_cgroup: None,
            escalator: Escalator::new(Thresholds::default()),
            pmu_targets,
            started,
            unavailable,
        }
    }

    /// Follows the pressure of the target's own cgroup (for example `/user.slice/app-game.scope`).
    /// A change of target replaces the sensor, so pressure of an old target never describes a new
    /// one; a target without a cgroup, or one whose pressure files are absent, simply has none.
    pub(super) fn track_target_cgroup(&mut self, cgroup_path: Option<&str>) {
        if self.tracked_cgroup.as_deref() == cgroup_path {
            return;
        }
        let now_ns = self.started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
        self.registry.deactivate("psi_cgroup", now_ns);
        self.tracked_cgroup = cgroup_path.map(str::to_owned);
        let Some(path) = cgroup_path else { return };
        let relative = path.trim_start_matches('/');
        // The path comes from procfs; never let it climb out of the cgroup mount.
        if relative.split('/').any(|part| part == ".." || part == ".") {
            return;
        }
        self.registry.register(Box::new(PsiSensor::cgroup(
            &self.cgroup_root.join(relative),
        )));
        if let Err(denied) = self.registry.activate("psi_cgroup", now_ns) {
            log::debug!(
                "sensor psi_cgroup unavailable for {path}: {}",
                describe(&denied)
            );
        }
    }

    /// The threads the PMU sensor would count if an escalation started it.
    pub(super) fn set_pmu_targets(&self, active: &crate::process_tree::TaskMap) {
        if let Ok(mut targets) = self.pmu_targets.lock() {
            *targets = Some((active.clone(), crate::metrics::TaskStatsMap::default()));
        }
    }

    /// Samples every running sensor now, then lets sustained pressure start a diagnostic sensor.
    pub(super) fn sample(&mut self) -> Vec<SensorReading> {
        let now_ns = self.started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
        let readings = self.registry.sample_all(now_ns);
        if let Some(trigger) = self.escalator.observe(&readings, now_ns) {
            log::info!(
                "sensor escalation: {:?} ({})",
                trigger.hypothesis,
                trigger.reason
            );
            match trigger.diagnostic_sensor {
                Some(id) => match self.registry.activate(id, now_ns) {
                    Ok(()) => log::info!("sensor {id} admitted for {:?}", trigger.hypothesis),
                    Err(denied) => log::info!("sensor {id} not admitted: {}", describe(&denied)),
                },
                None => log::info!("no diagnostic sensor exists for {:?}", trigger.hypothesis),
            }
        }
        readings
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
