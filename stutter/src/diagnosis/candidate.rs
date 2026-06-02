//! Diagnosis candidate scoring/finalization; this module owns ranking, thresholds, and missing-evidence messages.

use std::cmp::Ordering;

use super::{
    CandidateRejection, Confidence, Diagnosis, DiagnosisCandidate, DiagnosisConfig, EvidenceItem,
    EvidenceKind, StutterCause,
};

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

pub(super) fn push_candidate(
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

pub(super) fn push_supporting_evidence(
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

pub(super) fn scheduler_candidate_cause(candidates: &[DiagnosisCandidate]) -> Option<StutterCause> {
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
pub(super) struct DiagnosisContextSummary {
    pub(super) saw_scheduler_delay: bool,
    pub(super) saw_significant_irq: bool,
    pub(super) saw_high_gpu: bool,
    pub(super) saw_significant_block_io: bool,
    pub(super) saw_high_cpu_psi: bool,
}

pub(super) fn scheduler_delay_score(latency_ns: u64, config: DiagnosisConfig) -> f32 {
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

pub(super) fn is_scheduler_cause(cause: StutterCause) -> bool {
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
        evidence_chains: Vec::new(),
        primary: None,
        candidates,
        candidate_rejections: Vec::new(),
        summary,
    }
}

pub(super) fn finalize_diagnosis(
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
            evidence_chains: Vec::new(),
            primary: None,
            candidates: Vec::new(),
            candidate_rejections: Vec::new(),
            summary: "insufficient evidence: no candidate reached diagnosis thresholds".to_owned(),
        };
    }

    normalize_and_sort_candidates(&mut candidates);
    sort_candidates(&mut candidates);

    let primary = candidates[0].clone();
    if candidate_is_sufficient_primary(&primary, config).is_err() {
        let candidate_rejections = candidate_rejections(&candidates, config);
        let missing_evidence =
            missing_evidence_for_context(&candidates, Some(&primary), config, &context);
        let mut diagnosis = unknown_diagnosis_with_candidates(
            candidates,
            missing_evidence,
            format!(
                "insufficient evidence: best_candidate={:?} confidence={:?} score={:.2}",
                primary.cause, primary.confidence, primary.score
            ),
        );
        diagnosis.candidate_rejections = candidate_rejections;
        return diagnosis;
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
    let candidate_rejections = candidate_rejections(&candidates, config);

    Diagnosis {
        cause: primary.cause,
        confidence: primary.confidence,
        secondary_causes,
        evidence,
        missing_evidence,
        evidence_chains: Vec::new(),
        primary: Some(primary),
        candidates,
        candidate_rejections,
        summary,
    }
}

fn candidate_rejections(
    candidates: &[DiagnosisCandidate],
    config: DiagnosisConfig,
) -> Vec<CandidateRejection> {
    candidates
        .iter()
        .filter_map(|candidate| {
            let reasons = primary_rejection_reasons(candidate, config);
            (!reasons.is_empty()).then_some(CandidateRejection {
                cause: candidate.cause,
                score: candidate.score,
                confidence: candidate.confidence,
                reasons,
            })
        })
        .collect()
}
