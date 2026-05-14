use crate::{
    commands::input::ReleaseCheckCommandInput,
    release::{ReleaseReadinessReport, evaluate_release_readiness},
};

pub fn run_release_check_command(input: ReleaseCheckCommandInput) -> anyhow::Result<()> {
    let report = evaluate_release_readiness(input.channel, &input.inputs);

    if input.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", render_release_readiness_text(&report));
    }

    if input.enforce && !report.passed {
        anyhow::bail!("release readiness gates failed for {}", report.channel);
    }

    Ok(())
}

fn render_release_readiness_text(report: &ReleaseReadinessReport) -> String {
    let mut text = String::new();

    text.push_str("Release readiness\n");
    text.push_str("=================\n");
    text.push_str(&format!("channel: {}\n", report.channel));
    text.push_str(&format!("passed: {}\n", report.passed));
    text.push_str("gates:\n");
    for gate in &report.gates {
        text.push_str(&format!(
            "- {}: {} - {}\n",
            gate.code,
            if gate.passed { "passed" } else { "failed" },
            gate.description
        ));
    }
    text.push_str("changelog_categories: ");
    text.push_str(&report.changelog_categories.join(", "));
    text.push('\n');

    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::release::{ReleaseChannel, ReleaseReadinessInputs};

    #[test]
    fn release_readiness_text_shows_failed_gates_and_categories() {
        let report = evaluate_release_readiness(
            ReleaseChannel::LowRiskStable,
            &ReleaseReadinessInputs::default(),
        );

        let text = render_release_readiness_text(&report);

        assert!(text.contains("Release readiness"));
        assert!(text.contains("channel: low-risk-stable"));
        assert!(text.contains("soak_tests: failed"));
        assert!(text.contains("changelog_categories: safety"));
    }
}
