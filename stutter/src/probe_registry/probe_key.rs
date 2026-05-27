use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeKey {
    SchedulerRunnableLatency,
    IrqLatency,
    GpuHwmon,
    FrameLog,
    ForegroundWindow,
    BlockIo,
    CpuFreq,
    Faults,
    CpuPerf,
    PsiTimeline,
    PressureStallTimelineOverlay,
    RuntimeSlices,
    KmsPageflipTiming,
    DrmFenceLatency,
    WaylandPresentationTiming,
    DisplayTopology,
    DmaBufPathTracking,
    GpuEngineSampling,
    DirectScanoutStatus,
    DisplayPathCost,
    PerfCounterPresets,
    CompositorFramePacingViews,
}
