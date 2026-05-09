use serde::Serialize;

use crate::{
    diagnosis::{ClusterAnchorKind, Diagnosis},
    process_tree::TaskClass,
};

#[derive(Debug, Clone, Serialize, Default)]
pub(crate) struct SpikePoint {
    pub(crate) task: u32,
    pub(crate) class: TaskClass,
    pub(crate) process_pid: Option<u32>,
    pub(crate) comm: String,
    pub(crate) cpu: u32,
    pub(crate) wakeup_target_cpu: u32,
    pub(crate) latency_ns: u64,
    pub(crate) wakeup_ns: u64,
    pub(crate) switch_ns: u64,
    pub(crate) target_pending_wakeups: u32,
    pub(crate) observed_runnable_depth: u32,
    pub(crate) switch_prev_pid: u32,
    pub(crate) switch_prev_state: i64,
    pub(crate) switch_prev_state_label: String,
    pub(crate) elapsed_ms: Option<u64>,
    pub(crate) scx_ops: Option<String>,
    pub(crate) scx_state: Option<String>,
    pub(crate) waker_tid: u32,
    pub(crate) waker_comm: String,
    pub(crate) cause_tags: Vec<String>,
    pub(crate) primary_cause: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub(crate) struct SpikeCluster {
    pub(crate) points: Vec<SpikePoint>,
    pub(crate) distinct_tasks: usize,
    pub(crate) min_switch_ns: u64,
    pub(crate) max_switch_ns: u64,
    pub(crate) max_latency_ns: u64,
    pub(crate) diagnosis: Option<Diagnosis>,
    pub(crate) diagnosis_explanation: Option<DiagnosisExplanation>,
    pub(crate) anchor_task: Option<u32>,
    pub(crate) anchor_class: Option<TaskClass>,
    pub(crate) anchor_comm: Option<String>,
    pub(crate) anchor_kind: Option<ClusterAnchorKind>,
    pub(crate) foreground_pid: Option<u32>,
    pub(crate) foreground_app_id: Option<String>,
    pub(crate) foreground_class: Option<String>,
    pub(crate) foreground_confidence: Option<f32>,
    pub(crate) wake_graph: Vec<WakeGraphEdge>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct WakeGraphEdge {
    pub(crate) waker_tid: u32,
    pub(crate) waker_comm: String,
    pub(crate) wakee_tid: u32,
    pub(crate) wakee_comm: String,
    pub(crate) count: u64,
    pub(crate) max_latency_ns: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosisExplanation {
    pub primary_cause: Option<String>,
    pub primary_score: Option<f32>,
    pub primary_confidence: Option<String>,
    pub reason: String,
    pub evidence_items: Vec<DiagnosisEvidenceView>,
    pub competing_candidates: Vec<DiagnosisCandidateView>,
    pub missing_evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosisEvidenceView {
    pub kind: String,
    pub strength: f32,
    pub message: String,
    pub timestamp_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosisCandidateView {
    pub cause: String,
    pub score: f32,
    pub confidence: String,
    pub evidence_count: usize,
}
