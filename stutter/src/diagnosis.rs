use std::{cmp::Ordering, collections::BTreeMap};

use serde::{Deserialize, Serialize};

use crate::{
    metrics::format_latency,
    process_tree::TaskClass,
    recorder::{
        BlockIoRecord, CpuFreqRecord, GpuSample, IntervalRecord, IrqEventRecord,
        MigrationEventRecord,
    },
    report::{SpikeCluster, SpikePoint},
    session_io::RunArtifacts,
};

const IRQ_SIGNIFICANT_NS: u64 = 250_000; // start conservative
const BLOCK_IO_SIGNIFICANT_NS: u64 = 1_000_000;
const GPU_BUSY_BOUND_PERCENT: u32 = 95;
const SCHED_DELAY_SIGNIFICANT_NS: u64 = 2_000_000;

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
    pub low_ipc_threshold: f64,
    pub high_cache_mpki_threshold: f64,
    pub min_primary_score: f32,
    pub min_primary_confidence: Confidence,
    pub min_primary_evidence_items: usize,
    pub min_scheduler_latency_for_primary_ns: u64,
    pub min_non_scheduler_score_for_primary: f32,
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
            low_ipc_threshold: 0.75,
            high_cache_mpki_threshold: 30.0,
            min_primary_score: 0.40,
            min_primary_confidence: Confidence::Medium,
            min_primary_evidence_items: 1,
            min_scheduler_latency_for_primary_ns: SCHED_DELAY_SIGNIFICANT_NS,
            min_non_scheduler_score_for_primary: 0.40,
        }
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct DiagnosisThresholdDoc {
    pub key: &'static str,
    pub value: f64,
    pub unit: &'static str,
    pub description: &'static str,
}

impl DiagnosisConfig {
    #[allow(dead_code)]
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
            StutterCause::Unknown => 7,
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

#[allow(dead_code)]
fn confidence_threshold_value(confidence: Confidence) -> f64 {
    match confidence {
        Confidence::Low => 1.0,
        Confidence::Medium => 2.0,
        Confidence::High => 3.0,
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[allow(dead_code)]
pub enum EvidenceKind {
    SchedulerDelay,
    IrqOverlap,
    GpuBusy,
    BlockIo,
    CpuPressure,
    ScxState,
    CpuFrequency,
    Migration,
    PageFaults,
    CpuPerf,
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

fn is_compositor_point(p: &SpikePoint) -> bool {
    matches!(p.class, TaskClass::Compositor | TaskClass::GameScope)
}

fn is_game_point(p: &SpikePoint) -> bool {
    matches!(
        p.class,
        TaskClass::Game | TaskClass::GameHelper | TaskClass::WineServer
    ) || matches!(p.comm.as_str(), "Main" | "RenderThread")
}

fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

fn confidence_from_score(score: f32) -> Confidence {
    if score >= 0.75 {
        Confidence::High
    } else if score >= 0.40 {
        Confidence::Medium
    } else {
        Confidence::Low
    }
}

fn push_candidate(
    candidates: &mut Vec<DiagnosisCandidate>,
    cause: StutterCause,
    raw_score: f32,
    mut evidence: EvidenceItem,
) {
    let score = clamp01(raw_score);
    evidence.strength = clamp01(evidence.strength);

    if let Some(existing) = candidates.iter_mut().find(|c| c.cause == cause) {
        existing.score = existing.score.max(score);
        existing.confidence = confidence_from_score(existing.score);
        existing.evidence.push(evidence);
    } else {
        candidates.push(DiagnosisCandidate {
            cause,
            score,
            confidence: confidence_from_score(score),
            evidence: vec![evidence],
        });
    }
}

fn push_supporting_evidence(
    candidates: &mut [DiagnosisCandidate],
    cause: StutterCause,
    mut evidence: EvidenceItem,
) {
    evidence.strength = clamp01(evidence.strength);
    if let Some(candidate) = candidates
        .iter_mut()
        .find(|candidate| candidate.cause == cause)
    {
        candidate.evidence.push(evidence);
    }
}

fn scheduler_candidate_cause(candidates: &[DiagnosisCandidate]) -> Option<StutterCause> {
    candidates
        .iter()
        .filter(|candidate| {
            matches!(
                candidate.cause,
                StutterCause::GameThreadSchedulerDelay | StutterCause::CompositorSchedulerDelay
            )
        })
        .max_by(|a, b| {
            a.score
                .partial_cmp(&b.score)
                .unwrap_or(Ordering::Equal)
                .then_with(|| b.cause.priority().cmp(&a.cause.priority()))
        })
        .map(|candidate| candidate.cause)
}

#[derive(Default, Clone, Debug)]
struct DiagnosisContextSummary {
    saw_scheduler_delay: bool,
    saw_significant_irq: bool,
    saw_high_gpu: bool,
    saw_significant_block_io: bool,
    saw_high_cpu_psi: bool,
}

fn scheduler_delay_score(latency_ns: u64, config: DiagnosisConfig) -> f32 {
    if latency_ns < config.sched_delay_significant_ns {
        return 0.0;
    }

    let latency_ms = latency_ns as f32 / 1_000_000.0;
    let score = if latency_ms < 4.0 {
        0.40 + ((latency_ms - 2.0).max(0.0) / 2.0) * 0.15
    } else if latency_ms < 8.0 {
        0.55 + ((latency_ms - 4.0) / 4.0) * 0.20
    } else if latency_ms < 16.0 {
        0.75 + ((latency_ms - 8.0) / 8.0) * 0.25
    } else {
        1.0
    };
    score.clamp(0.40, 1.0)
}

fn is_scheduler_cause(cause: StutterCause) -> bool {
    matches!(
        cause,
        StutterCause::GameThreadSchedulerDelay | StutterCause::CompositorSchedulerDelay
    )
}

fn scheduler_latency_for_candidate(candidate: &DiagnosisCandidate) -> u64 {
    candidate
        .evidence
        .iter()
        .filter(|evidence| evidence.kind == EvidenceKind::SchedulerDelay)
        .filter_map(|evidence| Some(evidence.end_ns?.saturating_sub(evidence.start_ns?)))
        .max()
        .unwrap_or(0)
}

fn primary_rejection_reasons(
    candidate: &DiagnosisCandidate,
    config: DiagnosisConfig,
) -> Vec<String> {
    let mut reasons = Vec::new();

    if candidate.score < config.min_primary_score {
        reasons.push(format!(
            "candidate score below minimum primary score: {:.2} < {:.2}",
            candidate.score, config.min_primary_score
        ));
    }

    if candidate.confidence < config.min_primary_confidence {
        reasons.push(format!(
            "candidate confidence below minimum primary confidence: {:?} < {:?}",
            candidate.confidence, config.min_primary_confidence
        ));
    }

    if candidate.evidence.len() < config.min_primary_evidence_items {
        reasons.push(format!(
            "candidate has too few evidence items: {} < {}",
            candidate.evidence.len(),
            config.min_primary_evidence_items
        ));
    }

    if is_scheduler_cause(candidate.cause) {
        let scheduler_latency_ns = scheduler_latency_for_candidate(candidate);
        if scheduler_latency_ns < config.min_scheduler_latency_for_primary_ns {
            reasons.push(format!(
                "scheduler delay below primary threshold: {}ns < {}ns",
                scheduler_latency_ns, config.min_scheduler_latency_for_primary_ns
            ));
        }
    } else if config.min_non_scheduler_score_for_primary > config.min_primary_score
        && candidate.score < config.min_non_scheduler_score_for_primary
    {
        reasons.push(format!(
            "non-scheduler candidate score below minimum primary score: {:.2} < {:.2}",
            candidate.score, config.min_non_scheduler_score_for_primary
        ));
    }

    reasons
}

fn candidate_is_sufficient_primary(
    candidate: &DiagnosisCandidate,
    config: DiagnosisConfig,
) -> Result<(), String> {
    let reasons = primary_rejection_reasons(candidate, config);
    if let Some(reason) = reasons.into_iter().next() {
        Err(reason)
    } else {
        Ok(())
    }
}

fn push_unique_missing(missing: &mut Vec<String>, message: impl Into<String>) {
    let message = message.into();
    if !missing.iter().any(|existing| existing == &message) {
        missing.push(message);
    }
}

fn missing_evidence_for_context(
    candidates: &[DiagnosisCandidate],
    rejected_primary: Option<&DiagnosisCandidate>,
    config: DiagnosisConfig,
    context: &DiagnosisContextSummary,
) -> Vec<String> {
    let mut missing = Vec::new();

    if candidates.is_empty() {
        push_unique_missing(
            &mut missing,
            "no candidate reached the minimum evidence threshold",
        );
    }

    if let Some(candidate) = rejected_primary {
        for reason in primary_rejection_reasons(candidate, config) {
            push_unique_missing(&mut missing, reason);
        }
    }

    if !context.saw_scheduler_delay {
        push_unique_missing(&mut missing, "scheduler delay below primary threshold");
    }
    if !context.saw_significant_irq {
        push_unique_missing(
            &mut missing,
            "no IRQ event above significant-duration threshold",
        );
    }
    if !context.saw_high_gpu {
        push_unique_missing(&mut missing, "no GPU sample at or above busy threshold");
    }
    if !context.saw_significant_block_io {
        push_unique_missing(
            &mut missing,
            "no block I/O event above significant-duration threshold",
        );
    }
    if !context.saw_high_cpu_psi {
        push_unique_missing(&mut missing, "no CPU PSI sample above pressure threshold");
    }

    missing
}

fn normalize_and_sort_candidates(candidates: &mut [DiagnosisCandidate]) {
    for candidate in candidates {
        candidate.score = clamp01(candidate.score);
        candidate.confidence = confidence_from_score(candidate.score);
        for evidence in &mut candidate.evidence {
            evidence.strength = clamp01(evidence.strength);
        }
    }
}

fn sort_candidates(candidates: &mut [DiagnosisCandidate]) {
    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.cause.priority().cmp(&b.cause.priority()))
    });
}

fn unknown_diagnosis_with_candidates(
    candidates: Vec<DiagnosisCandidate>,
    missing_evidence: Vec<String>,
    summary: String,
) -> Diagnosis {
    let secondary_causes = candidates.iter().map(|c| c.cause).collect::<Vec<_>>();
    let evidence = candidates
        .iter()
        .flat_map(|c| c.evidence.iter().map(|e| e.message.clone()))
        .collect::<Vec<_>>();

    Diagnosis {
        cause: StutterCause::Unknown,
        confidence: Confidence::Low,
        secondary_causes,
        evidence,
        missing_evidence,
        primary: None,
        candidates,
        summary,
    }
}

fn finalize_diagnosis(
    mut candidates: Vec<DiagnosisCandidate>,
    config: DiagnosisConfig,
    context: DiagnosisContextSummary,
) -> Diagnosis {
    if candidates.is_empty() {
        let missing_evidence = missing_evidence_for_context(&[], None, config, &context);
        return Diagnosis {
            cause: StutterCause::Unknown,
            confidence: Confidence::Low,
            secondary_causes: Vec::new(),
            evidence: vec!["no strong correlation found".to_owned()],
            missing_evidence,
            primary: None,
            candidates: Vec::new(),
            summary: "insufficient evidence: no candidate reached diagnosis thresholds".to_owned(),
        };
    }

    normalize_and_sort_candidates(&mut candidates);
    sort_candidates(&mut candidates);

    let primary = candidates[0].clone();
    if candidate_is_sufficient_primary(&primary, config).is_err() {
        let missing_evidence =
            missing_evidence_for_context(&candidates, Some(&primary), config, &context);
        return unknown_diagnosis_with_candidates(
            candidates,
            missing_evidence,
            format!(
                "insufficient evidence: best_candidate={:?} confidence={:?} score={:.2}",
                primary.cause, primary.confidence, primary.score
            ),
        );
    }

    let secondary_causes = candidates
        .iter()
        .skip(1)
        .map(|c| c.cause)
        .collect::<Vec<_>>();

    let evidence = candidates
        .iter()
        .flat_map(|c| c.evidence.iter().map(|e| e.message.clone()))
        .collect::<Vec<_>>();

    let missing_evidence = missing_evidence_for_context(&candidates, None, config, &context);
    let summary = format!(
        "primary={:?} confidence={:?} score={:.2}",
        primary.cause, primary.confidence, primary.score
    );

    Diagnosis {
        cause: primary.cause,
        confidence: primary.confidence,
        secondary_causes,
        evidence,
        missing_evidence,
        primary: Some(primary),
        candidates,
        summary,
    }
}

pub(crate) fn select_anchor_for_diagnosis(
    cluster: &SpikeCluster,
    diagnosis: &Diagnosis,
) -> ClusterAnchor {
    if diagnosis.cause == StutterCause::GameThreadSchedulerDelay {
        let game_anchor = cluster
            .points
            .iter()
            .filter(|p| is_game_point(p) && p.latency_ns > SCHED_DELAY_SIGNIFICANT_NS)
            .max_by_key(|p| p.latency_ns);

        if let Some(p) = game_anchor {
            return ClusterAnchor {
                task: p.task,
                class: p.class,
                comm: p.comm.clone(),
                latency_ns: p.latency_ns,
                kind: ClusterAnchorKind::Game,
            };
        }
    }

    if diagnosis.cause == StutterCause::CompositorSchedulerDelay {
        let compositor_anchor = cluster
            .points
            .iter()
            .filter(|p| is_compositor_point(p) && p.latency_ns > SCHED_DELAY_SIGNIFICANT_NS)
            .max_by_key(|p| p.latency_ns);

        if let Some(p) = compositor_anchor {
            return ClusterAnchor {
                task: p.task,
                class: p.class,
                comm: p.comm.clone(),
                latency_ns: p.latency_ns,
                kind: ClusterAnchorKind::Compositor,
            };
        }
    }

    select_anchor(cluster)
}

pub(crate) fn select_anchor(cluster: &SpikeCluster) -> ClusterAnchor {
    // 1. Prefer highest-latency TaskClass::Compositor or TaskClass::GameScope point above 2ms.
    let compositor_anchor = cluster
        .points
        .iter()
        .filter(|p| is_compositor_point(p) && p.latency_ns > SCHED_DELAY_SIGNIFICANT_NS)
        .max_by_key(|p| p.latency_ns);

    if let Some(p) = compositor_anchor {
        return ClusterAnchor {
            task: p.task,
            class: p.class,
            comm: p.comm.clone(),
            latency_ns: p.latency_ns,
            kind: ClusterAnchorKind::Compositor,
        };
    }

    // 2. Else prefer highest-latency Game/RenderThread/Main point above 2ms.
    let game_anchor = cluster
        .points
        .iter()
        .filter(|p| is_game_point(p) && p.latency_ns > SCHED_DELAY_SIGNIFICANT_NS)
        .max_by_key(|p| p.latency_ns);

    if let Some(p) = game_anchor {
        return ClusterAnchor {
            task: p.task,
            class: p.class,
            comm: p.comm.clone(),
            latency_ns: p.latency_ns,
            kind: ClusterAnchorKind::Game,
        };
    }

    // 3. Else use highest-latency point.
    let fallback = cluster
        .points
        .iter()
        .max_by_key(|p| p.latency_ns)
        .expect("cluster must have points");

    ClusterAnchor {
        task: fallback.task,
        class: fallback.class,
        comm: fallback.comm.clone(),
        latency_ns: fallback.latency_ns,
        kind: ClusterAnchorKind::Other,
    }
}

pub fn diagnose_cluster(
    cluster: &SpikeCluster,
    artifacts: &RunArtifacts,
    window_ns: u64,
) -> Diagnosis {
    diagnose_cluster_with_config(cluster, artifacts, window_ns, DiagnosisConfig::default())
}

pub fn diagnose_cluster_with_config(
    cluster: &SpikeCluster,
    artifacts: &RunArtifacts,
    window_ns: u64,
    config: DiagnosisConfig,
) -> Diagnosis {
    // Compute cluster time window
    let start_ns = cluster.min_switch_ns.saturating_sub(window_ns);
    let end_ns = cluster.max_switch_ns.saturating_add(window_ns);

    // Helper: cluster elapsed and range based on spike point elapsed_ms
    let cluster_elapsed_opt = cluster.points.iter().filter_map(|p| p.elapsed_ms).min();
    let cluster_elapsed_range = {
        let mut it = cluster.points.iter().filter_map(|p| p.elapsed_ms);
        if let Some(first) = it.next() {
            let mut min = first;
            let mut max = first;
            for v in it {
                min = min.min(v);
                max = max.max(v);
            }
            Some((min, max))
        } else {
            None
        }
    };

    // Filter artifacts to the cluster window / elapsed range
    let irq_events: Vec<IrqEventRecord> = artifacts
        .irq_events
        .iter()
        .filter(|e| e.exit_ns >= start_ns && e.enter_ns <= end_ns)
        .cloned()
        .collect();

    let gpu_samples: Vec<GpuSample> = if let Some(elapsed) = cluster_elapsed_opt {
        artifacts
            .gpu_samples
            .iter()
            .filter(|s| s.elapsed_ms.abs_diff(elapsed) <= 50)
            .cloned()
            .collect()
    } else {
        Vec::new()
    };

    // frame events are not currently used in diagnosis, but could be considered
    // for future enhancements.

    let io_events: Vec<BlockIoRecord> = artifacts
        .block_io_events
        .iter()
        .filter(|e| {
            e.timestamp_ns >= start_ns && e.timestamp_ns.saturating_sub(e.duration_ns) <= end_ns
        })
        .cloned()
        .collect();

    let cpu_freq_events: Vec<CpuFreqRecord> = artifacts
        .cpu_freq_events
        .iter()
        .filter(|e| e.timestamp_ns >= start_ns && e.timestamp_ns <= end_ns)
        .cloned()
        .collect();

    let migration_events: Vec<MigrationEventRecord> = artifacts
        .migration_events
        .iter()
        .filter(|e| e.timestamp_ns >= start_ns && e.timestamp_ns <= end_ns)
        .cloned()
        .collect();

    let interval_records: Vec<IntervalRecord> = if let Some((min, max)) = cluster_elapsed_range {
        artifacts
            .intervals
            .iter()
            .filter(|r| {
                r.elapsed_ms >= min.saturating_sub(1000) && r.elapsed_ms <= max.saturating_add(1000)
            })
            .cloned()
            .collect()
    } else {
        Vec::new()
    };

    let mut candidates = Vec::new();
    let mut context = DiagnosisContextSummary::default();

    // 1. Compositor scheduler delay
    // compositor/gamescope >2ms near frame spike => CompositorSchedulerDelay
    let compositor_point = cluster
        .points
        .iter()
        .filter(|p| is_compositor_point(p) && p.latency_ns > config.sched_delay_significant_ns)
        .max_by_key(|p| p.latency_ns);

    if let Some(p) = compositor_point {
        context.saw_scheduler_delay = true;
        let score = scheduler_delay_score(p.latency_ns, config);
        let message = format!(
            "compositor thread '{}' delayed by {}",
            p.comm,
            format_latency(p.latency_ns)
        );
        push_candidate(
            &mut candidates,
            StutterCause::CompositorSchedulerDelay,
            score,
            EvidenceItem {
                kind: EvidenceKind::SchedulerDelay,
                strength: score,
                message,
                timestamp_ms: p.elapsed_ms,
                start_ns: Some(p.wakeup_ns),
                end_ns: Some(p.switch_ns),
            },
        );
        push_scx_evidence(&mut candidates, StutterCause::CompositorSchedulerDelay, p);
        push_runnable_depth_evidence(&mut candidates, StutterCause::CompositorSchedulerDelay, p);
    }

    // 2. Game thread scheduler delay
    // Game/Main/RenderThread >2ms near frame spike => GameThreadSchedulerDelay
    let game_point = cluster
        .points
        .iter()
        .filter(|p| is_game_point(p) && p.latency_ns > config.sched_delay_significant_ns)
        .max_by_key(|p| p.latency_ns);

    if let Some(p) = game_point {
        context.saw_scheduler_delay = true;
        let score = scheduler_delay_score(p.latency_ns, config);
        let message = format!(
            "game thread '{}' delayed by {}",
            p.comm,
            format_latency(p.latency_ns)
        );
        push_candidate(
            &mut candidates,
            StutterCause::GameThreadSchedulerDelay,
            score,
            EvidenceItem {
                kind: EvidenceKind::SchedulerDelay,
                strength: score,
                message,
                timestamp_ms: p.elapsed_ms,
                start_ns: Some(p.wakeup_ns),
                end_ns: Some(p.switch_ns),
            },
        );
        push_scx_evidence(&mut candidates, StutterCause::GameThreadSchedulerDelay, p);
        push_runnable_depth_evidence(&mut candidates, StutterCause::GameThreadSchedulerDelay, p);
    }

    // 3. IRQ overlap
    let max_irq = irq_events
        .iter()
        .filter(|e| e.duration_ns >= config.irq_significant_ns)
        .max_by_key(|e| e.duration_ns);

    if let Some(irq) = max_irq {
        context.saw_significant_irq = true;
        let mut score = irq.duration_ns as f32 / 2_000_000.0;
        if cluster.max_latency_ns > 0
            && (irq.duration_ns as f32 / cluster.max_latency_ns as f32) < 0.10
        {
            score = score.min(0.35);
        }
        let message = format!(
            "IRQ {} active during spike (max duration {})",
            irq.irq,
            format_latency(irq.duration_ns)
        );
        push_candidate(
            &mut candidates,
            StutterCause::IrqDelayCandidate,
            score,
            EvidenceItem {
                kind: EvidenceKind::IrqOverlap,
                strength: score,
                message,
                timestamp_ms: irq.elapsed_ms,
                start_ns: Some(irq.enter_ns),
                end_ns: Some(irq.exit_ns),
            },
        );
    }

    // 4. GPU bound
    let high_gpu = gpu_samples
        .iter()
        .filter_map(|s| {
            s.gpu_busy_percent
                .filter(|busy| *busy >= config.gpu_busy_bound_percent)
                .map(|busy| (s, busy))
        })
        .max_by_key(|(_, busy)| *busy);
    if let Some((sample, busy_percent)) = high_gpu {
        context.saw_high_gpu = true;
        let score = (busy_percent.saturating_sub(90)) as f32 / 10.0;
        let message = format!("GPU busy at {}%", busy_percent);
        push_candidate(
            &mut candidates,
            StutterCause::GpuBoundCandidate,
            score,
            EvidenceItem {
                kind: EvidenceKind::GpuBusy,
                strength: score,
                message,
                timestamp_ms: Some(sample.elapsed_ms),
                start_ns: None,
                end_ns: None,
            },
        );
    }

    // 5. Block I/O
    let max_io = io_events
        .iter()
        .filter(|e| e.duration_ns >= config.block_io_significant_ns)
        .max_by_key(|e| e.duration_ns);

    if let Some(io) = max_io {
        context.saw_significant_block_io = true;
        let score = (io.duration_ns as f32 / 8_000_000.0).max(0.40);
        let message = format!(
            "block I/O active (max duration {})",
            format_latency(io.duration_ns)
        );
        push_candidate(
            &mut candidates,
            StutterCause::BlockIoCandidate,
            score,
            EvidenceItem {
                kind: EvidenceKind::BlockIo,
                strength: score,
                message,
                timestamp_ms: Some(io.elapsed_ms),
                start_ns: Some(io.timestamp_ns.saturating_sub(io.duration_ns)),
                end_ns: Some(io.timestamp_ns),
            },
        );
    }

    // 6. CPU Pressure (PSI)
    let high_psi = interval_records
        .iter()
        .filter(|r| r.cpu_psi_some > config.cpu_psi_some_significant)
        .max_by(|a, b| {
            a.cpu_psi_some
                .partial_cmp(&b.cpu_psi_some)
                .unwrap_or(Ordering::Equal)
        });
    if let Some(r) = high_psi {
        context.saw_high_cpu_psi = true;
        let score = (r.cpu_psi_some as f32 / 100.0).max(0.40);
        let message = format!("high CPU PSI detected ({:.1}%)", r.cpu_psi_some);
        push_candidate(
            &mut candidates,
            StutterCause::CpuPressureCandidate,
            score,
            EvidenceItem {
                kind: EvidenceKind::CpuPressure,
                strength: score,
                message,
                timestamp_ms: Some(r.elapsed_ms),
                start_ns: None,
                end_ns: None,
            },
        );
    }

    if let Some(cause) = scheduler_candidate_cause(&candidates) {
        push_cpu_frequency_evidence(&mut candidates, cause, &cpu_freq_events, config);
        push_migration_evidence(
            &mut candidates,
            cause,
            &migration_events,
            cluster_elapsed_range,
            config,
        );
        push_page_fault_evidence(&mut candidates, cause, &interval_records, config);
        push_cpu_perf_evidence(&mut candidates, cause, &interval_records, cluster, config);
    }

    finalize_diagnosis(candidates, config, context)
}

fn push_runnable_depth_evidence(
    candidates: &mut [DiagnosisCandidate],
    cause: StutterCause,
    point: &SpikePoint,
) {
    if point.observed_runnable_depth >= 4 {
        push_supporting_evidence(
            candidates,
            cause,
            EvidenceItem {
                kind: EvidenceKind::SchedulerDelay,
                strength: 0.40,
                message: format!(
                    "high monitored runnable depth ({} target tasks) suggests target-local CPU contention",
                    point.observed_runnable_depth
                ),
                timestamp_ms: point.elapsed_ms,
                start_ns: Some(point.wakeup_ns),
                end_ns: Some(point.switch_ns),
            },
        );
    }

    if point.target_pending_wakeups >= 4 {
        push_supporting_evidence(
            candidates,
            cause,
            EvidenceItem {
                kind: EvidenceKind::SchedulerDelay,
                strength: 0.20,
                message: format!(
                    "monitored wakeup backlog ({} tasks) (diagnostic-only)",
                    point.target_pending_wakeups
                ),
                timestamp_ms: point.elapsed_ms,
                start_ns: Some(point.wakeup_ns),
                end_ns: Some(point.switch_ns),
            },
        );
    }
}

fn push_scx_evidence(
    candidates: &mut [DiagnosisCandidate],
    cause: StutterCause,
    point: &SpikePoint,
) {
    if point.scx_ops.is_none() && point.scx_state.is_none() {
        return;
    }

    push_supporting_evidence(
        candidates,
        cause,
        EvidenceItem {
            kind: EvidenceKind::ScxState,
            strength: 0.25,
            message: format!(
                "SCX state near spike: ops={} state={}",
                point.scx_ops.as_deref().unwrap_or("-"),
                point.scx_state.as_deref().unwrap_or("-")
            ),
            timestamp_ms: point.elapsed_ms,
            start_ns: Some(point.wakeup_ns),
            end_ns: Some(point.switch_ns),
        },
    );
}

fn push_cpu_frequency_evidence(
    candidates: &mut [DiagnosisCandidate],
    cause: StutterCause,
    events: &[CpuFreqRecord],
    config: DiagnosisConfig,
) {
    let mut by_cpu: BTreeMap<u32, (u32, u32)> = BTreeMap::new();
    for event in events {
        let entry = by_cpu
            .entry(event.cpu)
            .or_insert((event.freq_khz, event.freq_khz));
        entry.0 = entry.0.min(event.freq_khz);
        entry.1 = entry.1.max(event.freq_khz);
    }

    let Some((cpu, (min_freq, max_freq))) = by_cpu
        .into_iter()
        .filter(|(_, (_, max_freq))| *max_freq > 0)
        .map(|(cpu, (min_freq, max_freq))| {
            let drop_percent =
                ((max_freq.saturating_sub(min_freq)) as f64 / max_freq as f64) * 100.0;
            (cpu, (min_freq, max_freq, drop_percent))
        })
        .filter(|(_, (_, _, drop_percent))| *drop_percent >= config.cpu_freq_drop_percent)
        .max_by(|a, b| a.1.2.partial_cmp(&b.1.2).unwrap_or(Ordering::Equal))
        .map(|(cpu, (min_freq, max_freq, _))| (cpu, (min_freq, max_freq)))
    else {
        return;
    };

    push_supporting_evidence(
        candidates,
        cause,
        EvidenceItem {
            kind: EvidenceKind::CpuFrequency,
            strength: 0.25,
            message: format!(
                "CPU frequency drop near spike: cpu={} min={}kHz max={}kHz",
                cpu, min_freq, max_freq
            ),
            timestamp_ms: None,
            start_ns: None,
            end_ns: None,
        },
    );
}

fn push_migration_evidence(
    candidates: &mut [DiagnosisCandidate],
    cause: StutterCause,
    events: &[MigrationEventRecord],
    cluster_elapsed_range: Option<(u64, u64)>,
    config: DiagnosisConfig,
) {
    let Some(event) = events.iter().find(|event| {
        cluster_elapsed_range.is_none_or(|(min, max)| {
            event.elapsed_ms >= min.saturating_sub(config.migration_window_ms)
                && event.elapsed_ms <= max.saturating_add(config.migration_window_ms)
        })
    }) else {
        return;
    };

    push_supporting_evidence(
        candidates,
        cause,
        EvidenceItem {
            kind: EvidenceKind::Migration,
            strength: 0.20,
            message: format!(
                "task migration event near spike: tid={} cpu {}->{}",
                event.tid, event.from_cpu, event.to_cpu
            ),
            timestamp_ms: Some(event.elapsed_ms),
            start_ns: Some(event.timestamp_ns),
            end_ns: Some(event.timestamp_ns),
        },
    );
}

fn push_page_fault_evidence(
    candidates: &mut [DiagnosisCandidate],
    cause: StutterCause,
    intervals: &[IntervalRecord],
    config: DiagnosisConfig,
) {
    let Some(record) = intervals.iter().find(|record| {
        record.major_faults.saturating_add(record.minor_faults) >= config.page_fault_delta_threshold
    }) else {
        return;
    };

    push_supporting_evidence(
        candidates,
        cause,
        EvidenceItem {
            kind: EvidenceKind::PageFaults,
            strength: 0.20,
            message: "page fault activity near spike; treat as supporting evidence only".to_owned(),
            timestamp_ms: Some(record.elapsed_ms),
            start_ns: None,
            end_ns: None,
        },
    );
}

fn push_cpu_perf_evidence(
    candidates: &mut [DiagnosisCandidate],
    cause: StutterCause,
    intervals: &[IntervalRecord],
    cluster: &SpikeCluster,
    config: DiagnosisConfig,
) {
    let anchor = select_anchor(cluster);
    let anchor_process_pid = cluster
        .points
        .iter()
        .find(|point| point.task == anchor.task)
        .and_then(|point| point.process_pid);

    let mut matching = intervals
        .iter()
        .filter(|record| record.cpu_perf.is_some() && record.task == anchor.task)
        .collect::<Vec<_>>();

    if matching.is_empty()
        && let Some(process_pid) = anchor_process_pid
    {
        matching = intervals
            .iter()
            .filter(|record| record.cpu_perf.is_some() && record.process_pid == Some(process_pid))
            .collect();
    }

    if matching.is_empty() && is_cpu_perf_class(anchor.class) {
        matching = intervals
            .iter()
            .filter(|record| record.cpu_perf.is_some() && record.class == anchor.class)
            .collect();
    }

    let Some(record) = matching.into_iter().max_by(|a, b| {
        cpu_perf_evidence_score(a, config)
            .partial_cmp(&cpu_perf_evidence_score(b, config))
            .unwrap_or(Ordering::Equal)
    }) else {
        return;
    };

    let score = cpu_perf_evidence_score(record, config);
    if score <= 0.0 {
        return;
    }

    let Some(perf) = &record.cpu_perf else {
        return;
    };

    push_supporting_evidence(
        candidates,
        cause,
        EvidenceItem {
            kind: EvidenceKind::CpuPerf,
            strength: 0.25,
            message: format!(
                "CPU perf near spike: task={} ipc={} cache_mpki={} cache_miss_rate={}",
                record.task,
                format_optional_float(perf.ipc, 2),
                format_optional_float(perf.cache_mpki, 1),
                format_optional_percent(perf.cache_miss_rate),
            ),
            timestamp_ms: Some(record.elapsed_ms),
            start_ns: None,
            end_ns: None,
        },
    );
}

fn cpu_perf_evidence_score(record: &IntervalRecord, config: DiagnosisConfig) -> f64 {
    let Some(perf) = &record.cpu_perf else {
        return 0.0;
    };

    let low_ipc_score = perf
        .ipc
        .filter(|ipc| *ipc < config.low_ipc_threshold)
        .map(|ipc| 10_000.0 + config.low_ipc_threshold - ipc)
        .unwrap_or(0.0);
    if low_ipc_score > 0.0 {
        return low_ipc_score;
    }

    perf.cache_mpki
        .filter(|mpki| *mpki > config.high_cache_mpki_threshold)
        .map(|mpki| mpki - config.high_cache_mpki_threshold)
        .unwrap_or(0.0)
}

fn is_cpu_perf_class(class: TaskClass) -> bool {
    matches!(
        class,
        TaskClass::Game | TaskClass::GameHelper | TaskClass::WineServer | TaskClass::GameScope
    )
}

fn format_optional_float(value: Option<f64>, decimals: usize) -> String {
    let Some(value) = value.filter(|value| value.is_finite()) else {
        return "-".to_owned();
    };
    format!("{value:.decimals$}")
}

fn format_optional_percent(value: Option<f64>) -> String {
    value
        .filter(|value| value.is_finite())
        .map(|value| format!("{:.1}%", value * 100.0))
        .unwrap_or_else(|| "-".to_owned())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::{process_tree::TaskClass, report::SpikePoint};

    fn spike_point(task: u32, class: TaskClass, comm: &str, latency_ns: u64) -> SpikePoint {
        let switch_ns = 100_000_000 + u64::from(task);
        SpikePoint {
            task,
            class,
            process_pid: Some(task),
            comm: comm.to_owned(),
            latency_ns,
            wakeup_ns: switch_ns.saturating_sub(latency_ns),
            switch_ns,
            elapsed_ms: Some(100),
            ..Default::default()
        }
    }

    fn spike_cluster(points: Vec<SpikePoint>) -> SpikeCluster {
        let distinct_tasks = points
            .iter()
            .map(|point| point.task)
            .collect::<BTreeSet<_>>()
            .len();
        let min_switch_ns = points.iter().map(|p| p.switch_ns).min().unwrap_or(0);
        let max_switch_ns = points.iter().map(|p| p.switch_ns).max().unwrap_or(0);
        let max_latency_ns = points.iter().map(|p| p.latency_ns).max().unwrap_or(0);

        SpikeCluster {
            points,
            distinct_tasks,
            min_switch_ns,
            max_switch_ns,
            max_latency_ns,
            ..Default::default()
        }
    }

    fn irq_event(duration_ns: u64) -> IrqEventRecord {
        IrqEventRecord {
            elapsed_ms: Some(100),
            irq: 137,
            cpu: 0,
            enter_ns: 100_000_000,
            exit_ns: 100_000_000 + duration_ns,
            duration_ns,
        }
    }

    fn block_io_event(duration_ns: u64) -> BlockIoRecord {
        BlockIoRecord {
            elapsed_ms: 100,
            tid: 100,
            dev: 1,
            nr_sector: 8,
            sector: 2048,
            duration_ns,
            timestamp_ns: 100_500_000,
            rwbs: "R".to_owned(),
            ..Default::default()
        }
    }

    fn cpu_psi_interval(cpu_psi_some: f64) -> IntervalRecord {
        IntervalRecord {
            elapsed_ms: 100,
            task: 100,
            active: true,
            class: TaskClass::Unknown,
            comm: "worker-a".to_owned(),
            process_pid: Some(100),
            process_comm: "worker-a".into(),
            samples: 1,
            stored_samples: 1,
            cpu_psi_some,
            ..Default::default()
        }
    }

    fn candidate(diagnosis: &Diagnosis, cause: StutterCause) -> &DiagnosisCandidate {
        diagnosis
            .candidates
            .iter()
            .find(|candidate| candidate.cause == cause)
            .unwrap()
    }

    fn candidate_index(diagnosis: &Diagnosis, cause: StutterCause) -> Option<usize> {
        diagnosis
            .candidates
            .iter()
            .position(|candidate| candidate.cause == cause)
    }

    fn assert_no_candidate(diagnosis: &Diagnosis, cause: StutterCause) {
        assert!(
            !diagnosis
                .candidates
                .iter()
                .any(|candidate| candidate.cause == cause),
            "unexpected candidate {:?}: {:#?}",
            cause,
            diagnosis
        );
    }

    fn assert_missing_contains(diagnosis: &Diagnosis, needle: &str) {
        assert!(
            diagnosis
                .missing_evidence
                .iter()
                .any(|message| message.contains(needle)),
            "missing_evidence did not contain {:?}: {:?}",
            needle,
            diagnosis.missing_evidence
        );
    }

    #[test]
    fn confidence_words_are_cautious() {
        assert_eq!(Confidence::High.as_report_word(), "strong candidate");
        assert_eq!(Confidence::Medium.as_report_word(), "candidate");
        assert_eq!(Confidence::Low.as_report_word(), "weak candidate");

        assert!(Confidence::High.caution_text().contains("inference"));
        assert!(Confidence::Medium.caution_text().contains("mixed"));
        assert!(Confidence::Low.caution_text().contains("weak"));
    }

    #[test]
    fn diagnosis_threshold_table_covers_all_config_fields() {
        let table = DiagnosisConfig::default().threshold_table();
        let keys = table.iter().map(|entry| entry.key).collect::<BTreeSet<_>>();
        let expected = [
            "irq_significant_ns",
            "block_io_significant_ns",
            "gpu_busy_bound_percent",
            "sched_delay_significant_ns",
            "cpu_psi_some_significant",
            "cpu_freq_drop_percent",
            "migration_window_ms",
            "page_fault_delta_threshold",
            "low_ipc_threshold",
            "high_cache_mpki_threshold",
            "min_primary_score",
            "min_primary_confidence",
            "min_primary_evidence_items",
            "min_scheduler_latency_for_primary_ns",
            "min_non_scheduler_score_for_primary",
        ];

        assert_eq!(
            table.len(),
            keys.len(),
            "duplicate threshold keys: {table:?}"
        );
        for key in expected {
            assert!(keys.contains(key), "missing threshold key {key}");
        }
        assert_eq!(table.len(), expected.len());
        for entry in table {
            assert!(entry.value > 0.0, "non-positive threshold value: {entry:?}");
            assert!(
                !entry.description.trim().is_empty(),
                "missing threshold description: {entry:?}"
            );
        }
    }

    #[test]
    fn weak_scheduler_delay_is_not_forced_high_confidence() {
        let cluster = spike_cluster(vec![spike_point(
            456,
            TaskClass::Game,
            "RenderThread",
            2_100_000,
        )]);

        let d = diagnose_cluster(&cluster, &RunArtifacts::default(), 0);

        assert_eq!(d.cause, StutterCause::GameThreadSchedulerDelay);
        assert_ne!(d.confidence, Confidence::High);
        assert_eq!(
            candidate(&d, StutterCause::GameThreadSchedulerDelay).confidence,
            Confidence::Medium
        );
    }

    #[test]
    fn strict_primary_confidence_leaves_weak_scheduler_unknown() {
        let cluster = spike_cluster(vec![spike_point(
            456,
            TaskClass::Game,
            "RenderThread",
            2_100_000,
        )]);
        let config = DiagnosisConfig {
            min_primary_confidence: Confidence::High,
            ..DiagnosisConfig::default()
        };

        let d = diagnose_cluster_with_config(&cluster, &RunArtifacts::default(), 0, config);

        assert_eq!(d.cause, StutterCause::Unknown);
        assert!(d.primary.is_none());
        assert!(candidate_index(&d, StutterCause::GameThreadSchedulerDelay).is_some());
        assert_missing_contains(&d, "confidence below");
    }

    #[test]
    fn strong_scheduler_delay_still_reaches_high_confidence() {
        let cluster = spike_cluster(vec![spike_point(
            456,
            TaskClass::Game,
            "RenderThread",
            8_000_000,
        )]);

        let d = diagnose_cluster(&cluster, &RunArtifacts::default(), 0);

        assert_eq!(d.cause, StutterCause::GameThreadSchedulerDelay);
        assert_eq!(d.confidence, Confidence::High);
    }

    #[test]
    fn insufficient_evidence_returns_unknown_but_keeps_candidates() {
        let cluster = spike_cluster(vec![spike_point(
            123,
            TaskClass::Unknown,
            "worker",
            3_000_000,
        )]);
        let artifacts = RunArtifacts {
            irq_events: vec![irq_event(300_000)],
            ..Default::default()
        };

        let d = diagnose_cluster(&cluster, &artifacts, 0);

        assert_eq!(d.cause, StutterCause::Unknown);
        assert!(d.primary.is_none());
        assert!(candidate_index(&d, StutterCause::IrqDelayCandidate).is_some());
        assert!(
            d.secondary_causes
                .contains(&StutterCause::IrqDelayCandidate)
        );
        assert!(!d.evidence.is_empty());
        assert!(d.summary.starts_with("insufficient evidence"));
        assert_missing_contains(&d, "score below");
        assert_missing_contains(&d, "confidence below");
    }

    #[test]
    fn missing_evidence_is_serialized() {
        let cluster = spike_cluster(vec![spike_point(
            123,
            TaskClass::Unknown,
            "worker",
            3_000_000,
        )]);
        let artifacts = RunArtifacts {
            irq_events: vec![irq_event(300_000)],
            ..Default::default()
        };
        let d = diagnose_cluster(&cluster, &artifacts, 0);

        let json = serde_json::to_value(&d).unwrap();
        let missing = json
            .get("missing_evidence")
            .and_then(|value| value.as_array())
            .expect("missing_evidence should serialize as an array");

        assert!(
            missing
                .iter()
                .filter_map(|value| value.as_str())
                .any(|message| message.contains("score below")),
            "serialized missing_evidence={missing:?}"
        );
    }

    #[test]
    fn weak_irq_overlap_does_not_create_irq_candidate() {
        let config = DiagnosisConfig::default();
        let cluster = spike_cluster(vec![spike_point(
            123,
            TaskClass::Unknown,
            "worker",
            3_000_000,
        )]);
        let artifacts = RunArtifacts {
            irq_events: vec![irq_event(config.irq_significant_ns - 1)],
            ..Default::default()
        };

        let d = diagnose_cluster_with_config(&cluster, &artifacts, 0, config);

        assert_no_candidate(&d, StutterCause::IrqDelayCandidate);
        assert_eq!(d.cause, StutterCause::Unknown);
        assert!(d.primary.is_none());
        assert_missing_contains(&d, "no IRQ event");
    }

    #[test]
    fn weak_gpu_sample_does_not_create_gpu_candidate() {
        let config = DiagnosisConfig::default();
        let cluster = spike_cluster(vec![spike_point(
            123,
            TaskClass::Unknown,
            "worker",
            3_000_000,
        )]);
        let artifacts = RunArtifacts {
            gpu_samples: vec![GpuSample {
                elapsed_ms: 100,
                gpu_busy_percent: Some(config.gpu_busy_bound_percent - 1),
                ..Default::default()
            }],
            ..Default::default()
        };

        let d = diagnose_cluster_with_config(&cluster, &artifacts, 0, config);

        assert_no_candidate(&d, StutterCause::GpuBoundCandidate);
        assert_eq!(d.cause, StutterCause::Unknown);
        assert!(d.primary.is_none());
        assert_missing_contains(&d, "GPU");
    }

    #[test]
    fn weak_block_io_does_not_create_block_candidate() {
        let config = DiagnosisConfig::default();
        let cluster = spike_cluster(vec![spike_point(
            123,
            TaskClass::Unknown,
            "worker",
            3_000_000,
        )]);
        let artifacts = RunArtifacts {
            block_io_events: vec![block_io_event(config.block_io_significant_ns - 1)],
            ..Default::default()
        };

        let d = diagnose_cluster_with_config(&cluster, &artifacts, 0, config);

        assert_no_candidate(&d, StutterCause::BlockIoCandidate);
        assert_eq!(d.cause, StutterCause::Unknown);
        assert!(d.primary.is_none());
        assert_missing_contains(&d, "no block I/O");
    }

    #[test]
    fn weak_cpu_psi_does_not_create_cpu_pressure_candidate() {
        let config = DiagnosisConfig::default();
        let cluster = spike_cluster(vec![spike_point(
            123,
            TaskClass::Unknown,
            "worker",
            3_000_000,
        )]);
        let artifacts = RunArtifacts {
            intervals: vec![cpu_psi_interval(config.cpu_psi_some_significant - 1.0)],
            ..Default::default()
        };

        let d = diagnose_cluster_with_config(&cluster, &artifacts, 0, config);

        assert_no_candidate(&d, StutterCause::CpuPressureCandidate);
        assert_eq!(d.cause, StutterCause::Unknown);
        assert!(d.primary.is_none());
        assert_missing_contains(&d, "no CPU PSI");
    }

    #[test]
    fn below_threshold_scheduler_delay_does_not_create_scheduler_candidate() {
        let config = DiagnosisConfig::default();
        let cluster = spike_cluster(vec![spike_point(
            456,
            TaskClass::Game,
            "RenderThread",
            config.sched_delay_significant_ns - 1,
        )]);

        let d = diagnose_cluster_with_config(&cluster, &RunArtifacts::default(), 0, config);

        assert_no_candidate(&d, StutterCause::GameThreadSchedulerDelay);
        assert_eq!(d.cause, StutterCause::Unknown);
        assert!(d.primary.is_none());
        assert_missing_contains(&d, "scheduler delay below");
    }

    #[test]
    fn strong_scheduler_beats_weak_irq() {
        let cluster = spike_cluster(vec![spike_point(
            456,
            TaskClass::Game,
            "RenderThread",
            10_000_000,
        )]);
        let artifacts = RunArtifacts {
            irq_events: vec![irq_event(300_000)],
            ..Default::default()
        };

        let d = diagnose_cluster(&cluster, &artifacts, 0);

        assert_eq!(d.cause, StutterCause::GameThreadSchedulerDelay);
        assert!(candidate_index(&d, StutterCause::IrqDelayCandidate).is_some());
        assert!(
            candidate_index(&d, StutterCause::GameThreadSchedulerDelay).unwrap()
                < candidate_index(&d, StutterCause::IrqDelayCandidate).unwrap()
        );
    }

    #[test]
    fn strong_irq_beats_unknown_worker_noise() {
        let cluster = spike_cluster(vec![
            spike_point(100, TaskClass::Unknown, "worker-a", 3_000_000),
            spike_point(101, TaskClass::Unknown, "worker-b", 2_500_000),
            spike_point(102, TaskClass::Unknown, "worker-c", 2_000_000),
        ]);
        let artifacts = RunArtifacts {
            irq_events: vec![irq_event(4_000_000)],
            ..Default::default()
        };

        let d = diagnose_cluster(&cluster, &artifacts, 0);

        assert_eq!(d.cause, StutterCause::IrqDelayCandidate);
        assert_eq!(d.candidates[0].cause, StutterCause::IrqDelayCandidate);
    }

    #[test]
    fn block_io_orders_before_cpu_pressure_when_score_is_higher() {
        let cluster = spike_cluster(vec![
            spike_point(100, TaskClass::Unknown, "worker-a", 3_000_000),
            spike_point(101, TaskClass::Unknown, "worker-b", 2_500_000),
            spike_point(102, TaskClass::Unknown, "worker-c", 2_000_000),
        ]);
        let artifacts = RunArtifacts {
            block_io_events: vec![block_io_event(8_000_000)],
            intervals: vec![cpu_psi_interval(80.0)],
            ..Default::default()
        };

        let d = diagnose_cluster(&cluster, &artifacts, 0);

        assert_eq!(d.cause, StutterCause::BlockIoCandidate);
        assert_eq!(d.candidates[0].cause, StutterCause::BlockIoCandidate);
        assert!(candidate_index(&d, StutterCause::CpuPressureCandidate).is_some());
    }

    #[test]
    fn gpu_bound_beats_below_threshold_scheduler_delay() {
        let config = DiagnosisConfig::default();
        let cluster = spike_cluster(vec![spike_point(
            456,
            TaskClass::Game,
            "RenderThread",
            config.sched_delay_significant_ns - 1,
        )]);
        let artifacts = RunArtifacts {
            gpu_samples: vec![GpuSample {
                elapsed_ms: 100,
                gpu_busy_percent: Some(99),
                ..Default::default()
            }],
            ..Default::default()
        };

        let d = diagnose_cluster_with_config(&cluster, &artifacts, 0, config);

        assert_eq!(d.cause, StutterCause::GpuBoundCandidate);
        assert_no_candidate(&d, StutterCause::GameThreadSchedulerDelay);
    }

    #[test]
    fn diagnosis_report_summary_uses_candidate_wording() {
        let cluster = spike_cluster(vec![spike_point(
            456,
            TaskClass::Game,
            "RenderThread",
            8_000_000,
        )]);

        let d = diagnose_cluster(&cluster, &RunArtifacts::default(), 0);
        let summary = d.report_summary();

        assert!(summary.contains("GameThreadSchedulerDelay"));
        assert!(summary.contains("candidate"));
        assert!(summary.contains("inference"));
    }

    #[test]
    fn scx_secondary_evidence_keeps_scheduler_primary() {
        let mut point = spike_point(456, TaskClass::Game, "RenderThread", 8_000_000);
        point.scx_ops = Some("scx_lavd".to_owned());
        point.scx_state = Some("enabled".to_owned());
        let cluster = spike_cluster(vec![point]);

        let d = diagnose_cluster(&cluster, &RunArtifacts::default(), 0);

        assert_eq!(d.cause, StutterCause::GameThreadSchedulerDelay);
        let game = candidate(&d, StutterCause::GameThreadSchedulerDelay);
        assert!(
            game.evidence
                .iter()
                .any(|evidence| evidence.kind == EvidenceKind::ScxState)
        );
    }

    #[test]
    fn cpu_perf_is_supporting_scheduler_evidence() {
        let cluster = spike_cluster(vec![spike_point(
            456,
            TaskClass::Game,
            "RenderThread",
            8_000_000,
        )]);
        let artifacts = RunArtifacts {
            intervals: vec![IntervalRecord {
                elapsed_ms: 100,
                task: 456,
                active: true,
                class: TaskClass::Game,
                comm: "RenderThread".to_owned(),
                process_pid: Some(456),
                process_comm: "game".into(),
                samples: 1,
                stored_samples: 1,
                cpu_perf: Some(crate::metrics::CpuPerfRecord {
                    ipc: Some(0.50),
                    cache_mpki: Some(45.0),
                    cache_miss_rate: Some(0.10),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        };

        let d = diagnose_cluster(&cluster, &artifacts, 0);

        assert_eq!(d.cause, StutterCause::GameThreadSchedulerDelay);
        let game = candidate(&d, StutterCause::GameThreadSchedulerDelay);
        assert!(
            game.evidence
                .iter()
                .any(|evidence| evidence.kind == EvidenceKind::CpuPerf
                    && evidence.message.contains("ipc=0.50"))
        );
    }

    #[test]
    fn classifies_sway_compositor_delay_by_task_class_not_comm_name() {
        let cluster = spike_cluster(vec![spike_point(
            123,
            TaskClass::Compositor,
            "sway",
            3_000_000,
        )]);
        let d = diagnose_cluster(&cluster, &RunArtifacts::default(), 0);
        assert_eq!(d.cause, StutterCause::CompositorSchedulerDelay);
    }

    #[test]
    fn ranked_candidates_prefer_stronger_game_delay_over_weaker_compositor_delay() {
        let cluster = spike_cluster(vec![
            spike_point(123, TaskClass::Compositor, "sway", 3_000_000),
            spike_point(456, TaskClass::Game, "RenderThread", 10_000_000),
        ]);

        let d = diagnose_cluster(&cluster, &RunArtifacts::default(), 0);
        assert_eq!(d.cause, StutterCause::GameThreadSchedulerDelay);
        assert_eq!(
            d.primary.as_ref().unwrap().cause,
            StutterCause::GameThreadSchedulerDelay
        );
        assert!(
            d.secondary_causes
                .contains(&StutterCause::CompositorSchedulerDelay)
        );
        assert!(d.candidates.len() >= 2);

        let game = candidate(&d, StutterCause::GameThreadSchedulerDelay);
        let compositor = candidate(&d, StutterCause::CompositorSchedulerDelay);
        assert!(game.score > compositor.score);
        assert!(
            d.evidence
                .iter()
                .any(|e| e.contains("compositor thread 'sway' delayed by 3.000ms"))
        );
        assert!(
            d.evidence
                .iter()
                .any(|e| e.contains("game thread 'RenderThread' delayed by 10.000ms"))
        );
    }

    #[test]
    fn compositor_wins_when_compositor_and_game_scores_are_similar() {
        let cluster = spike_cluster(vec![
            spike_point(123, TaskClass::Compositor, "sway", 6_000_000),
            spike_point(456, TaskClass::Game, "RenderThread", 5_000_000),
        ]);

        let d = diagnose_cluster(&cluster, &RunArtifacts::default(), 0);
        assert_eq!(d.cause, StutterCause::CompositorSchedulerDelay);
    }

    #[test]
    fn ignores_tiny_irq_events() {
        let cluster = spike_cluster(vec![spike_point(
            123,
            TaskClass::Unknown,
            "other",
            3_000_000,
        )]);

        // 10us IRQ should be ignored (threshold 250us)
        let irq = irq_event(10_000);

        let artifacts = RunArtifacts {
            irq_events: vec![irq],
            ..Default::default()
        };
        let d = diagnose_cluster(&cluster, &artifacts, 0);
        assert!(
            !d.secondary_causes
                .contains(&StutterCause::IrqDelayCandidate)
        );
        assert_ne!(d.cause, StutterCause::IrqDelayCandidate);

        // 1ms IRQ should be caught
        let irq_big = irq_event(1_000_000);
        let artifacts2 = RunArtifacts {
            irq_events: vec![irq_big],
            ..Default::default()
        };
        let d2 = diagnose_cluster(&cluster, &artifacts2, 0);
        assert!(
            d2.secondary_causes
                .contains(&StutterCause::IrqDelayCandidate)
                || d2.cause == StutterCause::IrqDelayCandidate
        );
    }

    #[test]
    fn tiny_irq_is_low_or_absent_when_scheduler_delay_dominates() {
        let cluster = spike_cluster(vec![spike_point(
            456,
            TaskClass::Game,
            "RenderThread",
            8_000_000,
        )]);
        let irq = irq_event(300_000);
        let artifacts = RunArtifacts {
            irq_events: vec![irq],
            ..Default::default()
        };

        let d = diagnose_cluster(&cluster, &artifacts, 0);
        assert_eq!(d.cause, StutterCause::GameThreadSchedulerDelay);
        assert_ne!(d.cause, StutterCause::IrqDelayCandidate);

        if let Some(irq_candidate) = d
            .candidates
            .iter()
            .find(|candidate| candidate.cause == StutterCause::IrqDelayCandidate)
        {
            assert!(irq_candidate.score < 0.40 || irq_candidate.confidence == Confidence::Low);
        }
    }

    #[test]
    fn large_irq_can_be_primary_when_no_scheduler_anchor_exists() {
        let cluster = spike_cluster(vec![spike_point(
            123,
            TaskClass::Unknown,
            "other",
            3_000_000,
        )]);
        let irq = irq_event(4_000_000);
        let artifacts = RunArtifacts {
            irq_events: vec![irq],
            ..Default::default()
        };

        let d = diagnose_cluster(&cluster, &artifacts, 0);
        assert_eq!(d.cause, StutterCause::IrqDelayCandidate);
        assert!(matches!(
            d.confidence,
            Confidence::Medium | Confidence::High
        ));
        assert_eq!(
            d.primary.as_ref().unwrap().cause,
            StutterCause::IrqDelayCandidate
        );
    }

    #[test]
    fn diagnosis_keeps_legacy_fields_populated() {
        let cluster = spike_cluster(vec![spike_point(
            456,
            TaskClass::Game,
            "RenderThread",
            8_000_000,
        )]);

        let d = diagnose_cluster(&cluster, &RunArtifacts::default(), 0);
        assert_eq!(d.cause, StutterCause::GameThreadSchedulerDelay);
        assert_eq!(d.confidence, Confidence::High);
        assert!(!d.evidence.is_empty());
        assert!(d.primary.is_some());
        assert!(!d.candidates.is_empty());
        assert!(d.summary.contains("primary="));
    }

    #[test]
    fn cluster_anchor_follows_ranked_game_primary() {
        // compositor 3ms + game 10ms => diagnosis primary GameThreadSchedulerDelay and anchor_kind Game
        let cluster = spike_cluster(vec![
            spike_point(123, TaskClass::Compositor, "sway", 3_000_000),
            spike_point(456, TaskClass::Game, "RenderThread", 10_000_000),
        ]);

        let d = diagnose_cluster(&cluster, &RunArtifacts::default(), 0);
        assert_eq!(d.cause, StutterCause::GameThreadSchedulerDelay);

        let anchor = select_anchor_for_diagnosis(&cluster, &d);
        assert_eq!(anchor.kind, ClusterAnchorKind::Game);
        assert_eq!(anchor.task, 456);
    }

    #[test]
    fn cluster_anchor_follows_compositor_primary() {
        // compositor 6ms + game 5ms => diagnosis primary CompositorSchedulerDelay and anchor_kind Compositor
        let cluster = spike_cluster(vec![
            spike_point(123, TaskClass::Compositor, "sway", 6_000_000),
            spike_point(456, TaskClass::Game, "RenderThread", 5_000_000),
        ]);

        let d = diagnose_cluster(&cluster, &RunArtifacts::default(), 0);
        assert_eq!(d.cause, StutterCause::CompositorSchedulerDelay);

        let anchor = select_anchor_for_diagnosis(&cluster, &d);
        assert_eq!(anchor.kind, ClusterAnchorKind::Compositor);
        assert_eq!(anchor.task, 123);
    }
}
