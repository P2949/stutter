//! Diagnosis wording policy.
//!
//! Device-attribution wording is only allowed when the matching explicit
//! evidence chain was attached. This keeps reports from presenting
//! IRQ/GPU/fence/block-I/O candidates as causal conclusions when the
//! frame -> cluster -> event -> device -> recommendation chain is absent.

use super::Diagnosis;

pub(super) fn apply_causal_wording_policy(diagnosis: &mut Diagnosis) {
    let Some(primary) = diagnosis.primary.as_ref() else {
        return;
    };

    let cause = primary.cause;
    let confidence = primary.confidence;
    let score = primary.score;
    if !cause.requires_explicit_attribution_chain() {
        return;
    }

    if diagnosis.has_explicit_evidence_chain_for_cause(cause) {
        return;
    }

    let chain_label = cause.explicit_attribution_chain_label().unwrap_or("device");
    diagnosis.summary = format!(
        "candidate={cause:?} confidence={confidence:?} score={score:.2}; explicit {chain_label} evidence chain missing, so report wording is candidate-only"
    );

    push_unique(
        &mut diagnosis.missing_evidence,
        format!("explicit {chain_label} evidence chain required before causal attribution"),
    );
    push_unique(
        &mut diagnosis.evidence,
        format!(
            "causal wording policy: {cause:?} kept as a candidate, not an attribution, because no explicit {chain_label} chain was recorded"
        ),
    );
}

fn push_unique(values: &mut Vec<String>, value: impl Into<String>) {
    let value = value.into();
    if !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::diagnosis::{
        Confidence, DiagnosisCandidate, EvidenceChain, EvidenceChainKind, EvidenceChainNode,
        EvidenceChainNodeKind, EvidenceItem, EvidenceKind, StutterCause,
    };

    fn candidate(cause: StutterCause) -> DiagnosisCandidate {
        DiagnosisCandidate {
            cause,
            score: 0.90,
            confidence: Confidence::High,
            evidence: vec![EvidenceItem {
                kind: EvidenceKind::Unknown,
                strength: 0.90,
                message: "synthetic test evidence".to_owned(),
                timestamp_ms: Some(100),
                start_ns: Some(100_000_000),
                end_ns: Some(103_000_000),
            }],
        }
    }

    fn diagnosis_with_primary(cause: StutterCause) -> Diagnosis {
        let primary = candidate(cause);
        Diagnosis {
            cause,
            confidence: Confidence::High,
            secondary_causes: Vec::new(),
            evidence: vec!["synthetic primary evidence".to_owned()],
            missing_evidence: Vec::new(),
            evidence_chains: Vec::new(),
            primary: Some(primary.clone()),
            candidates: vec![primary],
            candidate_rejections: Vec::new(),
            summary: format!("primary={cause:?} confidence=High score=0.90"),
        }
    }

    fn explicit_chain(kind: EvidenceChainKind) -> EvidenceChain {
        EvidenceChain {
            kind,
            explicit: true,
            summary: "explicit synthetic chain".to_owned(),
            nodes: vec![
                EvidenceChainNode {
                    kind: EvidenceChainNodeKind::Frame,
                    label: "visible frame spike".to_owned(),
                    timestamp_ms: Some(100),
                    start_ns: None,
                    end_ns: None,
                    delta_from_previous_ms: None,
                    details: BTreeMap::new(),
                },
                EvidenceChainNode {
                    kind: EvidenceChainNodeKind::Cluster,
                    label: "scheduler spike cluster".to_owned(),
                    timestamp_ms: Some(100),
                    start_ns: Some(100_000_000),
                    end_ns: Some(103_000_000),
                    delta_from_previous_ms: Some(0),
                    details: BTreeMap::new(),
                },
                EvidenceChainNode {
                    kind: EvidenceChainNodeKind::Event,
                    label: "explicit event".to_owned(),
                    timestamp_ms: Some(100),
                    start_ns: Some(100_000_000),
                    end_ns: Some(103_000_000),
                    delta_from_previous_ms: Some(0),
                    details: BTreeMap::new(),
                },
                EvidenceChainNode {
                    kind: EvidenceChainNodeKind::Device,
                    label: "explicit device".to_owned(),
                    timestamp_ms: Some(100),
                    start_ns: Some(100_000_000),
                    end_ns: Some(103_000_000),
                    delta_from_previous_ms: Some(0),
                    details: BTreeMap::new(),
                },
                EvidenceChainNode {
                    kind: EvidenceChainNodeKind::Recommendation,
                    label: "explicit recommendation".to_owned(),
                    timestamp_ms: None,
                    start_ns: None,
                    end_ns: None,
                    delta_from_previous_ms: None,
                    details: BTreeMap::new(),
                },
            ],
        }
    }

    #[test]
    fn irq_primary_without_explicit_chain_is_downgraded_to_candidate_wording() {
        let mut diagnosis = diagnosis_with_primary(StutterCause::IrqDelayCandidate);

        apply_causal_wording_policy(&mut diagnosis);

        assert!(diagnosis.summary.contains("candidate=IrqDelayCandidate"));
        assert!(diagnosis.summary.contains("candidate-only"));
        assert!(
            diagnosis
                .missing_evidence
                .iter()
                .any(|item| item.contains("explicit IRQ evidence chain required"))
        );
        assert!(
            diagnosis
                .report_summary()
                .contains("explicit IRQ evidence chain missing")
        );
        assert!(!diagnosis.report_summary().contains("attributed to"));
        assert!(!diagnosis.report_summary().contains("caused by"));
    }

    #[test]
    fn block_io_primary_without_explicit_chain_is_downgraded_to_candidate_wording() {
        let mut diagnosis = diagnosis_with_primary(StutterCause::BlockIoCandidate);

        apply_causal_wording_policy(&mut diagnosis);

        assert!(diagnosis.summary.contains("candidate=BlockIoCandidate"));
        assert!(diagnosis.summary.contains("candidate-only"));
        assert!(
            diagnosis
                .missing_evidence
                .iter()
                .any(|item| item.contains("explicit block I/O evidence chain required"))
        );
    }

    #[test]
    fn gpu_primary_accepts_explicit_gpu_or_fence_chain() {
        let mut gpu_diagnosis = diagnosis_with_primary(StutterCause::GpuBoundCandidate);
        gpu_diagnosis
            .evidence_chains
            .push(explicit_chain(EvidenceChainKind::Gpu));
        let gpu_summary = gpu_diagnosis.summary.clone();

        apply_causal_wording_policy(&mut gpu_diagnosis);

        assert_eq!(gpu_diagnosis.summary, gpu_summary);

        let mut fence_diagnosis = diagnosis_with_primary(StutterCause::GpuBoundCandidate);
        fence_diagnosis
            .evidence_chains
            .push(explicit_chain(EvidenceChainKind::DrmFence));
        let fence_summary = fence_diagnosis.summary.clone();

        apply_causal_wording_policy(&mut fence_diagnosis);

        assert_eq!(fence_diagnosis.summary, fence_summary);
    }

    #[test]
    fn scheduler_primary_does_not_require_device_chain() {
        let mut diagnosis = diagnosis_with_primary(StutterCause::GameThreadSchedulerDelay);
        let original_summary = diagnosis.summary.clone();

        apply_causal_wording_policy(&mut diagnosis);

        assert_eq!(diagnosis.summary, original_summary);
        assert!(diagnosis.missing_evidence.is_empty());
    }
}
