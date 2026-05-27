use super::models::AdvisorReport;

pub fn render_advisor_report(report: &AdvisorReport) -> String {
    let mut out = String::new();
    pushln(&mut out, "# stutter advisor");
    pushln(&mut out, "");
    pushln(&mut out, format!("Run: {}", report.run.display()));
    pushln(&mut out, format!("Data quality: {:?}", report.data_quality));
    pushln(&mut out, format!("Verdict: {:?}", report.verdict));
    pushln(&mut out, "");
    pushln(&mut out, "## Recommendations");
    pushln(&mut out, "");
    for recommendation in &report.recommendations {
        pushln(&mut out, format!("- {}", recommendation.title));
        pushln(
            &mut out,
            format!("  rationale: {}", recommendation.rationale),
        );
        pushln(
            &mut out,
            format!("  confidence: {:?}", recommendation.confidence),
        );
        pushln(
            &mut out,
            format!("  safety: {}", recommendation.safety_note),
        );
        for command in &recommendation.suggested_commands {
            pushln(&mut out, format!("  command: {command}"));
        }
    }
    if report.recommendations.is_empty() {
        pushln(&mut out, "- none");
    }
    pushln(&mut out, "");
    pushln(&mut out, "## Warnings");
    pushln(&mut out, "");
    if report.warnings.is_empty() {
        pushln(&mut out, "- none");
    } else {
        for warning in &report.warnings {
            pushln(&mut out, format!("- {warning}"));
        }
    }
    out
}
fn pushln(out: &mut String, line: impl AsRef<str>) {
    out.push_str(line.as_ref());
    out.push('\n');
}
