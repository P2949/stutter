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
