use serde::{Deserialize, Serialize};

use crate::process_tree::TaskClass;

const IRQ_SIGNIFICANT_NS: u64 = 250_000; // start conservative
const BLOCK_IO_SIGNIFICANT_NS: u64 = 1_000_000;
const GPU_BUSY_BOUND_PERCENT: u32 = 95;
pub(super) const SCHED_DELAY_SIGNIFICANT_NS: u64 = 2_000_000;

#[derive(Clone, Copy, Debug)]
pub struct DiagnosisConfig {
    pub irq_significant_ns: u64,
    pub block_io_significant_ns: u64,
    pub gpu_busy_bound_percent: u32,
    pub sched_delay_significant_ns: u64,
    pub cpu_psi_some_significant: f64,
    pub cpu_freq_drop_percent: f64,
    pub migration_window_ms: u64,
    pub page_fault_delta_threshold: u64,
    pub frame_coincidence_window_ms: u64,
    pub frame_spike_frametime_ms: f64,
    pub low_ipc_threshold: f64,
    pub high_cache_mpki_threshold: f64,
    pub min_primary_score: f32,
    pub min_primary_confidence: Confidence,
    pub min_primary_evidence_items: usize,
    pub min_scheduler_latency_for_primary_ns: u64,
    pub min_non_scheduler_score_for_primary: f32,
    pub runtime_high_ratio: f64,
    pub runtime_wait_high_ratio: f64,
    pub runtime_min_samples_for_primary_support: usize,
}

impl Default for DiagnosisConfig {
    fn default() -> Self {
        Self {
            irq_significant_ns: IRQ_SIGNIFICANT_NS,
            block_io_significant_ns: BLOCK_IO_SIGNIFICANT_NS,
            gpu_busy_bound_percent: GPU_BUSY_BOUND_PERCENT,
            sched_delay_significant_ns: SCHED_DELAY_SIGNIFICANT_NS,
            cpu_psi_some_significant: 50.0,
            cpu_freq_drop_percent: 20.0,
            migration_window_ms: 5,
            page_fault_delta_threshold: 1,
            frame_coincidence_window_ms: 33,
            frame_spike_frametime_ms: 16.6,
            low_ipc_threshold: 0.75,
            high_cache_mpki_threshold: 30.0,
            min_primary_score: 0.40,
            min_primary_confidence: Confidence::Medium,
            min_primary_evidence_items: 1,
            min_scheduler_latency_for_primary_ns: SCHED_DELAY_SIGNIFICANT_NS,
            min_non_scheduler_score_for_primary: 0.40,
            runtime_high_ratio: 0.80,
            runtime_wait_high_ratio: 0.50,
            runtime_min_samples_for_primary_support: 3,
        }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct DiagnosisThresholdDoc {
    pub key: &'static str,
    pub value: f64,
    pub unit: &'static str,
    pub description: &'static str,
}

impl DiagnosisConfig {
    pub fn threshold_table(&self) -> Vec<DiagnosisThresholdDoc> {
        vec![
            DiagnosisThresholdDoc {
                key: "irq_significant_ns",
                value: self.irq_significant_ns as f64,
                unit: "ns",
                description: "Minimum IRQ handler duration considered meaningful overlap evidence.",
            },
            DiagnosisThresholdDoc {
                key: "block_io_significant_ns",
                value: self.block_io_significant_ns as f64,
                unit: "ns",
                description: "Minimum block I/O request duration considered meaningful overlap evidence.",
            },
            DiagnosisThresholdDoc {
                key: "gpu_busy_bound_percent",
                value: self.gpu_busy_bound_percent as f64,
                unit: "%",
                description: "GPU busy percentage considered bounded enough to create a GPU candidate.",
            },
            DiagnosisThresholdDoc {
                key: "sched_delay_significant_ns",
                value: self.sched_delay_significant_ns as f64,
                unit: "ns",
                description: "Minimum game or compositor scheduler delay considered candidate evidence.",
            },
            DiagnosisThresholdDoc {
                key: "cpu_psi_some_significant",
                value: self.cpu_psi_some_significant,
                unit: "%",
                description: "CPU PSI some percentage considered meaningful CPU pressure evidence.",
            },
            DiagnosisThresholdDoc {
                key: "cpu_freq_drop_percent",
                value: self.cpu_freq_drop_percent,
                unit: "%",
                description: "CPU frequency drop required before frequency data becomes supporting evidence.",
            },
            DiagnosisThresholdDoc {
                key: "migration_window_ms",
                value: self.migration_window_ms as f64,
                unit: "ms",
                description: "Elapsed-time window around a cluster for migration supporting evidence.",
            },
            DiagnosisThresholdDoc {
                key: "page_fault_delta_threshold",
                value: self.page_fault_delta_threshold as f64,
                unit: "faults",
                description: "Minimum page fault delta before fault data becomes supporting evidence.",
            },
            DiagnosisThresholdDoc {
                key: "frame_coincidence_window_ms",
                value: self.frame_coincidence_window_ms as f64,
                unit: "ms",
                description: "Elapsed-time window used to link frame-time spikes to scheduler clusters.",
            },
            DiagnosisThresholdDoc {
                key: "frame_spike_frametime_ms",
                value: self.frame_spike_frametime_ms,
                unit: "ms",
                description: "Frame time above which a frame is considered a visible frame spike for cluster evidence.",
            },
            DiagnosisThresholdDoc {
                key: "low_ipc_threshold",
                value: self.low_ipc_threshold,
                unit: "ipc",
                description: "IPC level below which CPU perf data becomes supporting evidence.",
            },
            DiagnosisThresholdDoc {
                key: "high_cache_mpki_threshold",
                value: self.high_cache_mpki_threshold,
                unit: "MPKI",
                description: "Cache misses per thousand instructions above which CPU perf data becomes supporting evidence.",
            },
            DiagnosisThresholdDoc {
                key: "min_primary_score",
                value: self.min_primary_score as f64,
                unit: "score",
                description: "Minimum normalized candidate score required before a candidate can be primary.",
            },
            DiagnosisThresholdDoc {
                key: "min_primary_confidence",
                value: confidence_threshold_value(self.min_primary_confidence),
                unit: "band",
                description: "Minimum confidence band required before a candidate can be primary.",
            },
            DiagnosisThresholdDoc {
                key: "min_primary_evidence_items",
                value: self.min_primary_evidence_items as f64,
                unit: "items",
                description: "Minimum number of evidence items required before a candidate can be primary.",
            },
            DiagnosisThresholdDoc {
                key: "min_scheduler_latency_for_primary_ns",
                value: self.min_scheduler_latency_for_primary_ns as f64,
                unit: "ns",
                description: "Minimum scheduler latency required before a scheduler candidate can be primary.",
            },
            DiagnosisThresholdDoc {
                key: "min_non_scheduler_score_for_primary",
                value: self.min_non_scheduler_score_for_primary as f64,
                unit: "score",
                description: "Minimum normalized score required before a non-scheduler candidate can be primary.",
            },
            DiagnosisThresholdDoc {
                key: "runtime_high_ratio",
                value: self.runtime_high_ratio,
                unit: "ratio",
                description: "Runtime-slice CPU time ratio considered high supporting evidence near a spike.",
            },
            DiagnosisThresholdDoc {
                key: "runtime_wait_high_ratio",
                value: self.runtime_wait_high_ratio,
                unit: "ratio",
                description: "Runtime-slice runqueue wait ratio considered high supporting evidence near a spike.",
            },
            DiagnosisThresholdDoc {
                key: "runtime_min_samples_for_primary_support",
                value: self.runtime_min_samples_for_primary_support as f64,
                unit: "samples",
                description: "Minimum runtime-slice samples required before runtime evidence may strengthen primary support.",
            },
        ]
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum StutterCause {
    CompositorSchedulerDelay,
    GameThreadSchedulerDelay,
    IrqDelayCandidate,
    GpuBoundCandidate,
    BlockIoCandidate,
    CpuPressureCandidate,
    CpuMonopolizationCandidate,
    RuntimeWaitCandidate,
    Unknown,
}

impl StutterCause {
    pub fn priority(&self) -> u32 {
        match self {
            StutterCause::CompositorSchedulerDelay => 1,
            StutterCause::GameThreadSchedulerDelay => 2,
            StutterCause::IrqDelayCandidate => 3,
            StutterCause::GpuBoundCandidate => 4,
            StutterCause::BlockIoCandidate => 5,
            StutterCause::CpuPressureCandidate => 6,
            StutterCause::CpuMonopolizationCandidate => 7,
            StutterCause::RuntimeWaitCandidate => 8,
            StutterCause::Unknown => 9,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Confidence {
    Low,
    Medium,
    High,
}

impl Confidence {
    pub fn as_report_word(self) -> &'static str {
        match self {
            Confidence::High => "strong candidate",
            Confidence::Medium => "candidate",
            Confidence::Low => "weak candidate",
        }
    }

    pub fn caution_text(self) -> &'static str {
        match self {
            Confidence::High => {
                "evidence is strong, but this is still a profiler inference rather than proof"
            }
            Confidence::Medium => {
                "evidence is mixed; treat this as a candidate cause and inspect secondary evidence"
            }
            Confidence::Low => {
                "evidence is weak; do not treat this as a reliable cause without more data"
            }
        }
    }
}

fn confidence_threshold_value(confidence: Confidence) -> f64 {
    match confidence {
        Confidence::Low => 1.0,
        Confidence::Medium => 2.0,
        Confidence::High => 3.0,
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum EvidenceKind {
    SchedulerDelay,
    IrqOverlap,
    GpuBusy,
    DrmFenceWait,
    BlockIo,
    CpuPressure,
    ScxState,
    CpuFrequency,
    Migration,
    PageFaults,
    CpuPerf,
    RuntimeSlice,
    RuntimeCpuUse,
    RuntimeRunqueueWait,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvidenceItem {
    pub kind: EvidenceKind,
    pub strength: f32,
    pub message: String,
    pub timestamp_ms: Option<u64>,
    pub start_ns: Option<u64>,
    pub end_ns: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosisCandidate {
    pub cause: StutterCause,
    pub score: f32,
    pub confidence: Confidence,
    pub evidence: Vec<EvidenceItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CandidateRejection {
    pub cause: StutterCause,
    pub score: f32,
    pub confidence: Confidence,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub enum ClusterAnchorKind {
    Compositor,
    Game,
    Other,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClusterAnchor {
    pub task: u32,
    pub class: TaskClass,
    pub comm: String,
    pub latency_ns: u64,
    pub kind: ClusterAnchorKind,
}

#[derive(Debug, Clone, Serialize)]
pub struct Diagnosis {
    pub cause: StutterCause, // Primary cause (for compatibility)
    pub confidence: Confidence,
    pub secondary_causes: Vec<StutterCause>,
    pub evidence: Vec<String>,
    #[serde(default)]
    pub missing_evidence: Vec<String>,
    pub primary: Option<DiagnosisCandidate>,
    pub candidates: Vec<DiagnosisCandidate>,
    #[serde(default)]
    pub candidate_rejections: Vec<CandidateRejection>,
    pub summary: String,
}

impl Diagnosis {
    pub fn report_summary(&self) -> String {
        match &self.primary {
            Some(primary) => format!(
                "{:?}: {} (confidence={:?}, score={:.2}) - {}",
                primary.cause,
                primary.confidence.as_report_word(),
                primary.confidence,
                primary.score,
                primary.confidence.caution_text()
            ),
            None => "Unknown: no strong correlation found".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FrameDiagnosis {
    pub frame_elapsed_ms: u64,
    pub frametime_ms: f64,
    pub diagnosis: Diagnosis,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LiveDiagnosisEntry {
    pub elapsed_ms: u64,
    pub cause: StutterCause,
    pub confidence: Confidence,
    pub anchor_class: TaskClass,
    pub anchor_comm: String,
    pub evidence: Vec<String>,
}
