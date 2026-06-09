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

    if !diagnosis.evidence_chains.is_empty() {
        pushln(&mut output, format!("{indent}explicit evidence chains:"));
        for chain in diagnosis.evidence_chains.iter().take(4) {
            pushln(
                &mut output,
                format!(
                    "{indent}  - kind={} explicit={} summary={}",
                    chain.kind, chain.explicit, chain.summary
                ),
            );
            for node in chain.nodes.iter().take(6) {
                let detail_text = if node.details.is_empty() {
                    String::new()
                } else {
                    format!(
                        " details={}",
                        node.details
                            .iter()
                            .map(|(key, value)| format!("{key}={value}"))
                            .collect::<Vec<_>>()
                            .join(",")
                    )
                };
                let delta = node
                    .delta_from_previous_ms
                    .map(|delta| format!(" delta_from_previous_ms={delta}"))
                    .unwrap_or_default();
                pushln(
                    &mut output,
                    format!(
                        "{indent}    -> {} label={} timestamp_ms={} start_ns={} end_ns={}{}{}",
                        node.kind,
                        node.label,
                        node.timestamp_ms
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "-".to_owned()),
                        node.start_ns
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "-".to_owned()),
                        node.end_ns
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "-".to_owned()),
                        delta,
                        detail_text
                    ),
                );
            }
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
