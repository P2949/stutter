//! Diagnosis bridge helpers for report analysis.
//!
//! Owns calling the diagnosis engine for spike clusters and translating diagnosis output for report
//! models. Does not own clustering, pressure timelines, task rows, or report orchestration.

use super::*;

pub(crate) fn perform_diagnosis(
    clusters: &mut [SpikeCluster],
    artifacts: &session_io::RunArtifacts,
    cluster_window_ns: u64,
) {
    for cluster in clusters {
        let diagnosis = diagnose_cluster(cluster, artifacts, cluster_window_ns);
        let diagnosis_explanation = explain_diagnosis(&diagnosis);
        let anchor = select_anchor_for_diagnosis(cluster, &diagnosis);
        cluster.anchor_task = Some(anchor.task);
        cluster.anchor_class = Some(anchor.class);
        cluster.anchor_comm = Some(anchor.comm);
        cluster.anchor_kind = Some(anchor.kind);
        cluster.diagnosis_explanation = Some(diagnosis_explanation);
        cluster.diagnosis = Some(diagnosis);
    }
}

pub(crate) fn explain_diagnosis(diagnosis: &Diagnosis) -> DiagnosisExplanation {
    let primary = diagnosis.primary.as_ref();
    let evidence_items = primary
        .map(|primary| {
            primary
                .evidence
                .iter()
                .map(|evidence| DiagnosisEvidenceView {
                    kind: format!("{:?}", evidence.kind),
                    strength: evidence.strength,
                    message: evidence.message.clone(),
                    timestamp_ms: evidence.timestamp_ms,
                })
                .collect()
        })
        .unwrap_or_default();

    let competing_candidates = diagnosis
        .candidates
        .iter()
        .skip(usize::from(primary.is_some()))
        .map(|candidate| DiagnosisCandidateView {
            cause: format!("{:?}", candidate.cause),
            score: candidate.score,
            confidence: format!("{:?}", candidate.confidence),
            evidence_count: candidate.evidence.len(),
        })
        .collect();

    DiagnosisExplanation {
        primary_cause: primary.map(|primary| format!("{:?}", primary.cause)),
        primary_score: primary.map(|primary| primary.score),
        primary_confidence: primary.map(|primary| format!("{:?}", primary.confidence)),
        reason: diagnosis.summary.clone(),
        evidence_items,
        competing_candidates,
        missing_evidence: diagnosis.missing_evidence.clone(),
    }
}
