//! Diagnosis wording policy.
//!
//! Device-attribution wording is only allowed when the matching explicit
//! evidence chain was attached. This keeps reports from presenting
//! IRQ/GPU/fence/block-I/O candidates as causal conclusions when the
//! frame -> cluster -> event -> device -> recommendation chain is absent.

use super::{CausalConfidence, Diagnosis, StutterCause};

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

/// Sets the separate causal dimension. It reuses the explicit-attribution-chain policy above:
/// device candidates stay `NotEstablished` without their chain, and nothing is established
/// without a visible-stutter linkage. Plain scheduler overlap is capped at `Medium`, other
/// context candidates at `Low`; the result never exceeds the observation confidence.
pub(super) fn apply_causal_confidence_policy(
    diagnosis: &mut Diagnosis,
    visible_stutter_linkage: bool,
) {
    diagnosis.causal_confidence = causal_confidence(diagnosis, visible_stutter_linkage);
}

fn causal_confidence(diagnosis: &Diagnosis, visible_stutter_linkage: bool) -> CausalConfidence {
    let Some(primary) = diagnosis.primary.as_ref() else {
        return CausalConfidence::NotEstablished;
    };
    if !visible_stutter_linkage {
        return CausalConfidence::NotEstablished;
    }
    let ceiling = if primary.cause.requires_explicit_attribution_chain() {
        if !diagnosis.has_explicit_evidence_chain_for_cause(primary.cause) {
            return CausalConfidence::NotEstablished;
        }
        CausalConfidence::High
    } else if matches!(
        primary.cause,
        StutterCause::CompositorSchedulerDelay | StutterCause::GameThreadSchedulerDelay
    ) {
        CausalConfidence::Medium
    } else {
        CausalConfidence::Low
    };
    CausalConfidence::from(primary.confidence).min(ceiling)
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
        CausalConfidence, Confidence, DiagnosisCandidate, EvidenceChain, EvidenceChainKind,
        EvidenceChainNode, EvidenceChainNodeKind, EvidenceItem, EvidenceKind, StutterCause,
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
            causal_confidence: CausalConfidence::NotEstablished,
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

    fn with_confidence(mut diagnosis: Diagnosis, confidence: Confidence) -> Diagnosis {
        diagnosis.confidence = confidence;
        diagnosis.primary.as_mut().unwrap().confidence = confidence;
        diagnosis
    }

    fn causal(diagnosis: &mut Diagnosis, linkage: bool) -> CausalConfidence {
        apply_causal_confidence_policy(diagnosis, linkage);
        diagnosis.causal_confidence
    }

    #[test]
    fn strong_delay_without_visible_linkage_keeps_observation_but_not_causality() {
        let mut diagnosis = diagnosis_with_primary(StutterCause::GameThreadSchedulerDelay);

        assert_eq!(
            causal(&mut diagnosis, false),
            CausalConfidence::NotEstablished
        );
        assert_eq!(diagnosis.confidence, Confidence::High);
        assert_eq!(diagnosis.primary.unwrap().confidence, Confidence::High);
    }

    #[test]
    fn device_overlap_without_attribution_chain_is_not_established() {
        for cause in [
            StutterCause::IrqDelayCandidate,
            StutterCause::BlockIoCandidate,
            StutterCause::GpuBoundCandidate,
        ] {
            let mut diagnosis = diagnosis_with_primary(cause);
            assert_eq!(
                causal(&mut diagnosis, true),
                CausalConfidence::NotEstablished,
                "{cause:?}"
            );
        }
    }

    #[test]
    fn explicit_chain_with_linkage_establishes_causality_within_observation_confidence() {
        for (cause, kind) in [
            (StutterCause::IrqDelayCandidate, EvidenceChainKind::Irq),
            (StutterCause::BlockIoCandidate, EvidenceChainKind::BlockIo),
            (StutterCause::GpuBoundCandidate, EvidenceChainKind::Gpu),
            (StutterCause::GpuBoundCandidate, EvidenceChainKind::DrmFence),
        ] {
            for observation in [Confidence::Low, Confidence::Medium, Confidence::High] {
                let mut diagnosis = with_confidence(diagnosis_with_primary(cause), observation);
                diagnosis.evidence_chains.push(explicit_chain(kind));

                let established = causal(&mut diagnosis, true);

                assert_eq!(
                    established,
                    CausalConfidence::from(observation),
                    "{cause:?}"
                );
                assert!(established <= CausalConfidence::from(observation));
            }
            // The same chain without a visible-stutter linkage establishes nothing.
            let mut diagnosis = diagnosis_with_primary(cause);
            diagnosis.evidence_chains.push(explicit_chain(kind));
            assert_eq!(
                causal(&mut diagnosis, false),
                CausalConfidence::NotEstablished
            );
        }
    }

    #[test]
    fn overlap_alone_never_produces_high_causal_confidence() {
        // Scheduler overlap with a coincident frame spike is temporal coincidence: capped.
        let mut scheduler = diagnosis_with_primary(StutterCause::CompositorSchedulerDelay);
        assert_eq!(causal(&mut scheduler, true), CausalConfidence::Medium);
        // Context candidates are capped lower still.
        for cause in [
            StutterCause::CpuPressureCandidate,
            StutterCause::CpuMonopolizationCandidate,
            StutterCause::RuntimeWaitCandidate,
        ] {
            let mut diagnosis = diagnosis_with_primary(cause);
            assert_eq!(
                causal(&mut diagnosis, true),
                CausalConfidence::Low,
                "{cause:?}"
            );
        }
        // No primary: nothing to establish.
        let mut unknown = diagnosis_with_primary(StutterCause::Unknown);
        unknown.primary = None;
        assert_eq!(causal(&mut unknown, true), CausalConfidence::NotEstablished);
    }

    #[test]
    fn causal_confidence_never_exceeds_observation_confidence() {
        for cause in [
            StutterCause::GameThreadSchedulerDelay,
            StutterCause::IrqDelayCandidate,
            StutterCause::CpuPressureCandidate,
        ] {
            for observation in [Confidence::Low, Confidence::Medium, Confidence::High] {
                for linkage in [false, true] {
                    let mut diagnosis = with_confidence(diagnosis_with_primary(cause), observation);
                    diagnosis
                        .evidence_chains
                        .push(explicit_chain(EvidenceChainKind::Irq));
                    assert!(causal(&mut diagnosis, linkage) <= CausalConfidence::from(observation));
                }
            }
        }
    }
}
