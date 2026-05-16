use crate::daemon::{DaemonAcceptanceReport, run_fake_daemon_acceptance_suite};

pub fn run_acceptance_command(
    input: crate::commands::input::DaemonAcceptanceCommandInput,
) -> anyhow::Result<()> {
    let report = run_fake_daemon_acceptance_suite();

    if input.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", render_acceptance_text(&report));
    }

    if !report.passed {
        anyhow::bail!("daemon acceptance suite failed one or more steps");
    }

    Ok(())
}

pub fn render_acceptance_text(report: &DaemonAcceptanceReport) -> String {
    let mut text = String::new();

    text.push_str("Daemon acceptance\n");
    text.push_str("=================\n");
    text.push_str(&format!("suite: {}\n", report.suite));
    text.push_str(&format!("passed: {}\n", report.passed));

    for step in &report.steps {
        text.push_str(&format!(
            "step {} {}: {} - {}\n",
            step.number,
            step.code,
            if step.passed { "passed" } else { "failed" },
            step.evidence
        ));
    }

    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_acceptance_text_lists_final_boss_steps() {
        let report = crate::daemon::run_fake_daemon_acceptance_suite();

        let text = render_acceptance_text(&report);

        assert!(text.contains("Daemon acceptance"));
        assert!(text.contains("suite: fake-daemon-100-percent-acceptance"));
        assert!(text.contains("passed: true"));
        assert!(text.contains("step 1 install_service: passed"));
        assert!(text.contains("step 22 complete_audit_history: passed"));
    }
}
