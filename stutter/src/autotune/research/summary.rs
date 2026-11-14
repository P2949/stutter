//! Deterministic human summary of a research report (PLAN section 7.2). It restates the sealed
//! report, states what was not measured, and never presents an unqualified numeric difference
//! as a Proton or Stutter win.

use std::fmt::Write;

use super::report::ResearchReportV1;

/// Statuses that carry a numeric comparison but not real-game qualification.
fn is_unqualified(status: &str) -> bool {
    status == "unqualified_provenance" || status == "native_context_consistent_unqualified"
}

pub fn render_summary(report: &ResearchReportV1) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# Cyberpunk x Proton research summary\n");
    let _ = writeln!(out, "Intent digest: `{}`\n", report.intent_sha256);
    let _ = writeln!(
        out,
        "Attempts: {} scheduled, {} sealed, {} completed. Control: {:?}.\n",
        report.attempts_scheduled,
        report.attempts_sealed,
        report.attempts_completed,
        report.control
    );
    let _ = writeln!(out, "## Per-arm results\n");
    for arm in &report.arms {
        let _ = write!(
            out,
            "- {:?}: status `{}`, {} paired runs",
            arm.source, arm.status, arm.pairs
        );
        match (
            arm.mean_effect_percent,
            arm.ci_low_percent,
            arm.ci_high_percent,
        ) {
            (Some(mean), Some(low), Some(high)) => {
                let _ = write!(out, ", mean effect {mean:.2}% (CI {low:.2}% to {high:.2}%)");
            }
            _ => {
                let _ = write!(out, ", no numeric result");
            }
        }
        if let Some(verdict) = &arm.numeric_verdict {
            let _ = write!(out, ", numeric verdict `{verdict}`");
        }
        if is_unqualified(&arm.status) {
            let _ = write!(out, " (not a qualified result)");
        }
        let _ = writeln!(out);
    }
    let _ = writeln!(out, "\n## Not measured\n");
    let _ = writeln!(
        out,
        "- Input-to-photon latency and optical display timing: no hardware source exists.\n\
         - Anything absent from the sealed attempts above; missing data is unknown, not zero.\n\
         - Stutter's own decisions and worker transactions are reported separately, not here."
    );
    if report.arms.iter().any(|arm| is_unqualified(&arm.status)) {
        let _ = writeln!(
            out,
            "\nAt least one comparison lacks independent process, Steam and worker provenance, so \
             no best Proton configuration is claimed."
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autotune::research::{ProtonSource, report::ArmResult};

    fn report(status: &str, numeric: bool) -> ResearchReportV1 {
        ResearchReportV1 {
            schema_version: 2,
            intent_sha256: "a".repeat(64),
            attempts_scheduled: 12,
            attempts_sealed: 12,
            attempts_completed: 11,
            control: ProtonSource::ValveStable,
            arms: vec![ArmResult {
                source: ProtonSource::Ge,
                status: status.into(),
                numeric_verdict: numeric.then(|| "improved".to_owned()),
                pairs: 6,
                mean_effect_percent: numeric.then_some(10.0),
                ci_low_percent: numeric.then_some(9.0),
                ci_high_percent: numeric.then_some(11.0),
                rule_sha256: None,
            }],
        }
    }

    #[test]
    fn an_unqualified_gain_is_never_called_a_win() {
        let text = render_summary(&report("unqualified_provenance", true));
        assert!(text.contains("not a qualified result"));
        assert!(text.contains("no best Proton configuration is claimed"));
        assert!(text.contains("Input-to-photon"));
        assert_eq!(
            text,
            render_summary(&report("unqualified_provenance", true))
        );
    }

    #[test]
    fn refusals_state_no_numeric_result() {
        let text = render_summary(&report("refused_TooFewPairs", false));
        assert!(text.contains("no numeric result"));
        assert!(!text.contains("claimed"));
    }
}
