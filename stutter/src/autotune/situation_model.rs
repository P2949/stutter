//! Situation Classification v2 vocabulary. The flat `SituationKind` stays as the derived
//! compatibility/UI/artifact label; classification is modelled as independent dimensions so that
//! workload identity, pressure and diagnosis are not conflated.
use serde::{Deserialize, Serialize};

use crate::autotune::situation::SituationKind;

/// What the user is doing, independent of what is going wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkloadKind {
    Game,
    Compile,
    Browser,
    Media,
    Recording,
    VirtualMachine,
    Desktop,
    Idle,
    Unknown,
}

/// A resource under pressure. Direct telemetry can establish it without any diagnosis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PressureKind {
    Scheduler,
    Cpu,
    Gpu,
    Io,
    Irq,
    Memory,
    Thermal,
    Power,
}

impl SituationKind {
    /// The workload dimension of this situation.
    pub fn workload(self) -> WorkloadKind {
        match self {
            Self::GameFocused | Self::GameCpuSchedulerPressure | Self::GameGpuBound => {
                WorkloadKind::Game
            }
            Self::CompileLoad | Self::CompileCpuBound | Self::CompileLinkerPressure => {
                WorkloadKind::Compile
            }
            Self::BrowserFocused
            | Self::BrowserCpuPressure
            | Self::BrowserGpuVideo
            | Self::BrowserIoPressure => WorkloadKind::Browser,
            Self::MediaPlayback => WorkloadKind::Media,
            Self::Recording => WorkloadKind::Recording,
            Self::VirtualMachineLoad => WorkloadKind::VirtualMachine,
            Self::CompositorPressure
            | Self::CpuPressure
            | Self::IoPressure
            | Self::IrqPressure
            | Self::ThermalOrPowerLimit => WorkloadKind::Desktop,
            Self::Idle => WorkloadKind::Idle,
            Self::Unknown => WorkloadKind::Unknown,
        }
    }

    /// The pressure dimension of this situation, when it names one.
    pub fn pressure(self) -> Option<PressureKind> {
        match self {
            Self::GameCpuSchedulerPressure | Self::CompositorPressure => {
                Some(PressureKind::Scheduler)
            }
            Self::CpuPressure | Self::BrowserCpuPressure | Self::CompileCpuBound => {
                Some(PressureKind::Cpu)
            }
            Self::GameGpuBound | Self::BrowserGpuVideo => Some(PressureKind::Gpu),
            Self::IoPressure | Self::BrowserIoPressure | Self::CompileLinkerPressure => {
                Some(PressureKind::Io)
            }
            Self::IrqPressure => Some(PressureKind::Irq),
            Self::ThermalOrPowerLimit => Some(PressureKind::Thermal),
            Self::GameFocused
            | Self::CompileLoad
            | Self::BrowserFocused
            | Self::MediaPlayback
            | Self::Recording
            | Self::VirtualMachineLoad
            | Self::Idle
            | Self::Unknown => None,
        }
    }
}

/// How a pressure hypothesis was established.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PressureSource {
    /// Trusted telemetry alone; no diagnosis is involved.
    Direct,
    /// A diagnosis explained or corroborated the pressure.
    Diagnosis,
}

/// One pressure hypothesis and the signal that established it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PressureEvidence {
    pub kind: PressureKind,
    pub source: PressureSource,
    pub signal: &'static str,
    pub value: String,
    pub weight: f32,
}

/// Weight of a pressure established by direct telemetry. A diagnosis can raise confidence above it.
pub const DIRECT_PRESSURE_WEIGHT: f32 = 0.90;

/// Scheduler-pressure threshold: the diagnosis scheduler threshold calibrated in the v2 study.
const SCHEDULER_PRESSURE_P99_NS: u64 = 2_000_000;
/// Same trigger the GPU-power provider uses for a busy GPU.
const GPU_BUSY_PERCENT: u32 = 90;

/// Pressure hypotheses supported by direct signals only, with no diagnosis required. Each trigger
/// mirrors a trigger an existing provider or calibrated threshold already uses; none is new.
pub fn direct_pressure_evidence(
    signals: &crate::autotune::objective::ObjectiveSignals,
) -> Vec<PressureEvidence> {
    let mut pressures = Vec::new();
    let mut push = |kind, signal: &'static str, value: String| {
        pressures.push(PressureEvidence {
            kind,
            source: PressureSource::Direct,
            signal,
            value,
            weight: DIRECT_PRESSURE_WEIGHT,
        });
    };

    if let (Some(p99), Some(samples)) = (signals.runnable_latency_p99_ns, signals.runnable_samples)
        && samples >= crate::autotune::objective::MINIMUM_RUNNABLE_SAMPLES
        && p99 >= SCHEDULER_PRESSURE_P99_NS
    {
        push(
            PressureKind::Scheduler,
            "runnable_latency_p99_ns",
            p99.to_string(),
        );
    }
    if signals.gpu_power_limited == Some(true) {
        push(PressureKind::Gpu, "gpu_power_limited", "true".to_owned());
    } else if let Some(busy) = signals
        .gpu_busy_percent
        .filter(|busy| *busy >= GPU_BUSY_PERCENT)
    {
        push(PressureKind::Gpu, "gpu_busy_percent", busy.to_string());
    }
    if let Some(count) = signals.block_io_overlap_count.filter(|count| *count > 0) {
        push(
            PressureKind::Io,
            "block_io_overlap_count",
            count.to_string(),
        );
    }
    if let Some(count) = signals.irq_overlap_count.filter(|count| *count > 0) {
        push(PressureKind::Irq, "irq_overlap_count", count.to_string());
    }
    if let Some(events) = signals.swap_activity_events.filter(|events| *events > 0) {
        push(
            PressureKind::Memory,
            "swap_activity_events",
            events.to_string(),
        );
    } else if let Some(spikes) = signals.mem_stall_spike_count.filter(|spikes| *spikes > 0) {
        push(
            PressureKind::Memory,
            "mem_stall_spike_count",
            spikes.to_string(),
        );
    } else if let Some(events) = signals.dirty_writeback_events.filter(|events| *events > 0) {
        push(
            PressureKind::Memory,
            "dirty_writeback_events",
            events.to_string(),
        );
    }
    if signals.thermal_degraded == Some(true) {
        push(PressureKind::Thermal, "thermal_degraded", "true".to_owned());
    } else if let Some(count) = signals.thermal_throttle_count.filter(|count| *count > 0) {
        push(
            PressureKind::Thermal,
            "thermal_throttle_count",
            count.to_string(),
        );
    }
    if signals.cpu_power_limited == Some(true) {
        push(PressureKind::Power, "cpu_power_limited", "true".to_owned());
    }
    pressures
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_situation_has_a_workload_and_pressure_situations_name_their_pressure() {
        assert_eq!(
            SituationKind::GameCpuSchedulerPressure.workload(),
            WorkloadKind::Game
        );
        assert_eq!(
            SituationKind::GameCpuSchedulerPressure.pressure(),
            Some(PressureKind::Scheduler)
        );
        assert_eq!(SituationKind::GameFocused.pressure(), None);
        assert_eq!(
            SituationKind::CompileLinkerPressure.workload(),
            WorkloadKind::Compile
        );
        assert_eq!(
            SituationKind::CompileLinkerPressure.pressure(),
            Some(PressureKind::Io)
        );
        assert_eq!(SituationKind::Idle.workload(), WorkloadKind::Idle);
    }
}
