mod artifacts;
mod cluster;
mod correlation;
mod diagnosis;
mod display;
mod foreground;
mod frame;
mod header;
mod pressure;
mod quality;
mod regression;
mod root;
mod spike_point;
mod wake_graph;

pub use artifacts::{ArtifactsSummary, SpikeClusterSource, SpikeDensityBucket};
pub use cluster::{MIN_CLUSTER_TASKS, SpikeCluster, SpikeClusterAnalysis};
pub use correlation::{TextReportCorrelationSection, TextReportCorrelationSections};
pub use diagnosis::{
    Diagnosis, DiagnosisCandidate, DiagnosisEvidence, DiagnosisPrimary, DiagnosisRejection,
};
pub use display::{
    CrossGpuFenceCandidate, CrossGpuFenceSummary, DirectScanoutSummary, DisplayPathComponent,
    DisplayPathDiagnosisSummary, DmaBufPathSummary, DrmFenceTimingSummary, DrmFenceWaitSummary,
    EvidenceQuality, GpuEngineActivitySummary, KmsTimingSummary, ScanoutWindowEstimate,
    WaylandPresentationSummary,
};
pub use foreground::{FocusReportSummary, ForegroundReportSummary};
pub use frame::FrameDiagnosis;
pub use header::ReportHeaderSummary;
pub use pressure::{
    PressureKind, PressurePeakWindow, PressureTimelineCoverage, PressureTimelineSummary,
    PressureWindow,
};
pub use quality::{DataQualityLevel, DataQualitySummary};
pub use regression::RegressionMetric;
pub use root::ReportModel;
pub use spike_point::SpikePoint;
pub use wake_graph::WakeGraphEdge;

#[cfg(test)]
mod tests;
