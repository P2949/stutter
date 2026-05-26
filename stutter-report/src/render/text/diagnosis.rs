use super::pushln;
use crate::model::Diagnosis;

pub fn render_diagnosis_lines(diagnosis: &Diagnosis, indent: &str) -> String {
    let mut output = String::new();
    pushln(
        &mut output,
        format!("{indent}diagnosis: {}", diagnosis.report_summary()),
    );
    output.push_str(&render_diagnosis_detail_lines(diagnosis, indent));
    output.trim_end().to_owned()
}

pub fn render_diagnosis_detail_lines(diagnosis: &Diagnosis, indent: &str) -> String {
    let mut output = String::new();
    if !diagnosis.secondary_causes.is_empty() {
        pushln(
            &mut output,
            format!(
                "{indent}diagnosis_secondary causes={:?}",
                diagnosis.secondary_causes
            ),
        );
    }

    pushln(
        &mut output,
        format!("{indent}why this diagnosis was chosen:"),
    );
    if let Some(primary) = &diagnosis.primary {
        pushln(
            &mut output,
            format!(
                "{indent}  - primary={} confidence={} score={:.2}",
                primary.cause, primary.confidence, primary.score
            ),
        );
        for evidence in primary.evidence.iter().take(6) {
            pushln(
                &mut output,
                format!(
                    "{indent}  - evidence kind={} strength={:.2} msg={}",
                    evidence.kind, evidence.strength, evidence.message
                ),
            );
        }
    } else {
        pushln(
            &mut output,
            format!("{indent}  - no primary candidate met the reporting threshold"),
        );
    }

    pushln(
        &mut output,
        format!("{indent}evidence missing / not strong enough:"),
    );
    if diagnosis.missing_evidence.is_empty() {
        pushln(&mut output, format!("{indent}  - none recorded"));
    } else {
        for missing in diagnosis.missing_evidence.iter().take(6) {
            pushln(&mut output, format!("{indent}  - {missing}"));
        }
    }

    if !diagnosis.candidate_rejections.is_empty() {
        pushln(&mut output, format!("{indent}why not primary:"));
        for rejection in diagnosis.candidate_rejections.iter().take(3) {
            pushln(
                &mut output,
                format!(
                    "{indent}  - {} score={:.2} confidence={}",
                    rejection.cause, rejection.score, rejection.confidence
                ),
            );
            for reason in rejection.reasons.iter().take(3) {
                pushln(&mut output, format!("{indent}    - {reason}"));
            }
        }
    }

    pushln(&mut output, format!("{indent}diagnosis candidates:"));
    for candidate in diagnosis.candidates.iter().take(3) {
        pushln(
            &mut output,
            format!(
                "{indent}  - diagnosis_candidate cause={} confidence={} score={:.2} evidence_items={}",
                candidate.cause,
                candidate.confidence,
                candidate.score,
                candidate.evidence.len()
            ),
        );
    }

    if diagnosis.candidates.is_empty() {
        pushln(&mut output, format!("{indent}  - none recorded"));
    }

    output.trim_end().to_owned()
}
