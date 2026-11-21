//! Thermal context of an attempt: the CPU package and GPU edge temperatures read from the kernel's own
//! hwmon files when the attempt launched and when its result arrived. Two runs that start hot and cold,
//! or a run that ends near a throttle point, are not comparable measurements, so a study whose attempts
//! do not share a thermal context is not one comparison. Nothing here measures or controls anything: a
//! reading the host cannot take stays unknown, and an unknown is never read as cool.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// Attempts of one study must start within this spread of each other (millidegrees C).
pub const MAX_START_SPREAD_MC: i32 = 10_000;
/// An attempt that ends within this margin of the sensor's own critical temperature (`temp1_crit`, read from the
/// hardware: 100 C for this CPU, 110 C for this GPU) is treated as thermally limited. Where a sensor reports
/// no limit, these fallbacks apply.
pub const LIMIT_MARGIN_MC: i32 = 5_000;
pub const CPU_CEILING_MC: i32 = 95_000;
pub const GPU_CEILING_MC: i32 = 100_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThermalReading {
    pub cpu_mc: Option<i32>,
    pub gpu_mc: Option<i32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThermalContext {
    pub start: ThermalReading,
    pub end: ThermalReading,
    /// The sensors' own critical temperatures when the attempt launched (absent from older records).
    #[serde(default)]
    pub limit: ThermalReading,
}

fn temperature(dir: &Path, file: &str) -> Option<i32> {
    let value: i32 = std::fs::read_to_string(dir.join(file))
        .ok()?
        .trim()
        .parse()
        .ok()?;
    // Millidegrees; anything outside a sane range is a bad reading, not a temperature.
    (0..=150_000).contains(&value).then_some(value)
}

/// Read the CPU package (`coretemp`/`k10temp`) and GPU (`amdgpu`) temperatures under a hwmon root.
pub fn read(hwmon_root: &Path) -> ThermalReading {
    read_file(hwmon_root, "temp1_input")
}

/// The same sensors' own critical temperatures.
pub fn read_limits(hwmon_root: &Path) -> ThermalReading {
    read_file(hwmon_root, "temp1_crit")
}

fn read_file(hwmon_root: &Path, file: &str) -> ThermalReading {
    let mut reading = ThermalReading::default();
    let Ok(entries) = std::fs::read_dir(hwmon_root) else {
        return reading;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        let Ok(name) = std::fs::read_to_string(dir.join("name")) else {
            continue;
        };
        match name.trim() {
            "coretemp" | "k10temp" => reading.cpu_mc = reading.cpu_mc.or(temperature(&dir, file)),
            "amdgpu" => reading.gpu_mc = reading.gpu_mc.or(temperature(&dir, file)),
            _ => {}
        }
    }
    reading
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThermalState {
    /// No attempt recorded a thermal context (results from before it existed): nothing to hold them to.
    NotRecorded,
    Consistent,
    /// The attempts did not share a thermal context, or one ended thermally limited.
    Drifted,
    /// Some attempts recorded one and some did not, or a reading was missing.
    Unproven,
}

/// Judge a study's attempts together.
pub fn state(contexts: &[Option<ThermalContext>]) -> ThermalState {
    if contexts.iter().all(Option::is_none) {
        return ThermalState::NotRecorded;
    }
    let Some(all) = contexts.iter().copied().collect::<Option<Vec<_>>>() else {
        return ThermalState::Unproven;
    };
    let spread = |value: fn(&ThermalReading) -> Option<i32>| -> Option<(i32, i32)> {
        let temps: Option<Vec<i32>> = all.iter().map(|c| value(&c.start)).collect();
        let temps = temps?;
        Some((*temps.iter().min()?, *temps.iter().max()?))
    };
    let ended_limited = all.iter().any(|c| {
        let ceiling =
            |limit: Option<i32>, fallback: i32| limit.map_or(fallback, |l| l - LIMIT_MARGIN_MC);
        c.end
            .cpu_mc
            .is_some_and(|t| t >= ceiling(c.limit.cpu_mc, CPU_CEILING_MC))
            || c.end
                .gpu_mc
                .is_some_and(|t| t >= ceiling(c.limit.gpu_mc, GPU_CEILING_MC))
    });
    let (cpu, gpu) = (spread(|r| r.cpu_mc), spread(|r| r.gpu_mc));
    let drifted = ended_limited
        || [cpu, gpu]
            .into_iter()
            .flatten()
            .any(|(low, high)| high - low > MAX_START_SPREAD_MC);
    if drifted {
        ThermalState::Drifted
    } else if cpu.is_none() || gpu.is_none() {
        ThermalState::Unproven
    } else {
        ThermalState::Consistent
    }
}

#[cfg(test)]
#[path = "thermal_tests.rs"]
mod tests;
