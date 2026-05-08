#![allow(dead_code)]

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControllerPhase {
    Disabled,
    Observing,
    Planning,
    Applying,
    Measuring,
    Keeping,
    Reverting,
    Cooldown,
    Faulted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AutotuneMode {
    Observe,
    Suggest,
    ApplyLowRisk,
    ApplyMediumRisk,
    ApplyHighRisk,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SituationKind {
    Unknown,
    Idle,
    GameFocused,
    GameCpuSchedulerPressure,
    GameGpuBound,
    CompositorPressure,
    CpuPressure,
    IoPressure,
    IrqPressure,
    ThermalOrPowerLimit,
    CompileLoad,
    BrowserFocused,
    BrowserCpuPressure,
    BrowserGpuVideo,
    BrowserIoPressure,
    CompileCpuBound,
    CompileLinkerPressure,
    MediaPlayback,
    Recording,
    VirtualMachineLoad,
}
