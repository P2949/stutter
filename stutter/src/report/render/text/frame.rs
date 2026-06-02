use super::*;

pub(super) fn render_frame_diagnoses(frame_diagnoses: &[FrameDiagnosis], top: usize) -> String {
    let mut writer = ReportTextWriter::new();

    if frame_diagnoses.is_empty() {
        return writer.finish();
    }

    writer.line("frame spike diagnoses");
    writer.line("---------------------");
    for (rank, diag) in frame_diagnoses.iter().take(top).enumerate() {
        let mapped_diag = stutter_report::model::FrameDiagnosis {
            frame_elapsed_ms: diag.frame_elapsed_ms,
            frametime_ms: diag.frametime_ms,
            diagnosis: map_diagnosis(&diag.diagnosis),
        };
        writer.line(stutter_report::render::text::frame::render_frame_diagnosis(
            rank + 1,
            &mapped_diag,
        ));
    }
    writer.blank();
    writer.finish()
}

pub(super) fn map_diagnosis(d: &crate::diagnosis::Diagnosis) -> stutter_report::model::Diagnosis {
    stutter_report::model::Diagnosis {
        primary: d
            .primary
            .as_ref()
            .map(|p| stutter_report::model::DiagnosisPrimary {
                cause: format!("{:?}", p.cause),
                confidence: format!("{:?}", p.confidence),
                score: p.score,
                evidence: p.evidence.iter().map(map_diagnosis_evidence).collect(),
            }),
        candidates: d
            .candidates
            .iter()
            .map(|c| stutter_report::model::DiagnosisCandidate {
                cause: format!("{:?}", c.cause),
                confidence: format!("{:?}", c.confidence),
                score: c.score,
                evidence: c.evidence.iter().map(map_diagnosis_evidence).collect(),
            })
            .collect(),
        missing_evidence: d.missing_evidence.clone(),
        evidence_chains: d.evidence_chains.iter().map(map_evidence_chain).collect(),
        candidate_rejections: d
            .candidate_rejections
            .iter()
            .map(|r| stutter_report::model::DiagnosisRejection {
                cause: format!("{:?}", r.cause),
                score: r.score,
                confidence: format!("{:?}", r.confidence),
                reasons: r.reasons.clone(),
            })
            .collect(),
        secondary_causes: d
            .secondary_causes
            .iter()
            .map(|s| format!("{:?}", s))
            .collect(),
        report_summary: d.report_summary().to_owned(),
    }
}

fn map_diagnosis_evidence(
    evidence: &crate::diagnosis::EvidenceItem,
) -> stutter_report::model::DiagnosisEvidence {
    stutter_report::model::DiagnosisEvidence {
        kind: format!("{:?}", evidence.kind),
        strength: evidence.strength,
        message: evidence.message.clone(),
    }
}

fn map_evidence_chain(
    chain: &crate::diagnosis::EvidenceChain,
) -> stutter_report::model::DiagnosisEvidenceChain {
    stutter_report::model::DiagnosisEvidenceChain {
        kind: format!("{:?}", chain.kind),
        explicit: chain.explicit,
        summary: chain.summary.clone(),
        nodes: chain
            .nodes
            .iter()
            .map(|node| stutter_report::model::DiagnosisEvidenceChainNode {
                kind: format!("{:?}", node.kind),
                label: node.label.clone(),
                timestamp_ms: node.timestamp_ms,
                start_ns: node.start_ns,
                end_ns: node.end_ns,
                delta_from_previous_ms: node.delta_from_previous_ms,
                details: node.details.clone(),
            })
            .collect(),
    }
}
