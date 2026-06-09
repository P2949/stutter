use crate::daemon::testing::{DaemonSoakReport, run_fake_daemon_soak};

pub fn run_soak_command(
    input: crate::commands::input::DaemonSoakCommandInput,
) -> anyhow::Result<()> {
    let report = run_fake_daemon_soak(&input.config);

    if input.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", render_soak_text(&report));
    }

    if !report.passed {
        anyhow::bail!("daemon fake soak exceeded one or more budgets");
    }

    Ok(())
}

pub fn render_soak_text(report: &DaemonSoakReport) -> String {
    let mut text = String::new();

    text.push_str("Daemon scenario soak\n");
    text.push_str("====================\n");
    text.push_str(&format!("profile: {}\n", report.profile));
    text.push_str(&format!("duration_seconds: {}\n", report.duration_seconds));
    text.push_str(&format!("ticks: {}\n", report.ticks));
    text.push_str(&format!("passed: {}\n", report.passed));
    text.push_str(&format!(
        "scenario_count: {}\n",
        report.metrics.scenario_count
    ));
    text.push_str(&format!(
        "planner_decisions: {}\n",
        report.metrics.planner_decisions
    ));
    text.push_str(&format!(
        "memory_growth_bytes: {}\n",
        report.metrics.memory_growth_bytes
    ));
    text.push_str(&format!(
        "disk_growth_bytes: {}\n",
        report.metrics.disk_growth_bytes
    ));
    text.push_str(&format!(
        "max_event_queue_len: {}\n",
        report.metrics.max_event_queue_len
    ));
    text.push_str(&format!("task_count: {}\n", report.metrics.task_count));
    text.push_str(&format!(
        "history_bytes: {}\n",
        report.metrics.history_bytes
    ));
    text.push_str(&format!(
        "cpu_millis_per_second: {}\n",
        report.metrics.cpu_millis_per_second
    ));
    text.push_str(&format!(
        "wakeups_per_second: {}\n",
        report.metrics.wakeups_per_second
    ));
    text.push_str(&format!("event_drops: {}\n", report.metrics.event_drops));
    text.push_str(&format!(
        "fake_actions_started: {}\n",
        report.metrics.fake_actions_started
    ));
    text.push_str(&format!(
        "fake_rollbacks: {}\n",
        report.metrics.fake_rollbacks
    ));
    text.push_str(&format!(
        "max_active_experiments: {}\n",
        report.metrics.max_active_experiments
    ));
    for scenario in &report.scenarios {
        text.push_str(&format!(
            "scenario: {} mode={} ticks={} passed={} decisions={}\n",
            scenario.name,
            scenario.mode,
            scenario.ticks,
            scenario.passed,
            scenario.decisions.join(",")
        ));
    }
    for failure in &report.failures {
        text.push_str(&format!(
            "failure: {} - {}\n",
            failure.reason_code, failure.message
        ));
    }

    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_soak_text_contains_budget_metrics() {
        let config = crate::daemon::testing::DaemonSoakConfig::default();
        let report = crate::daemon::testing::run_fake_daemon_soak(&config);

        let text = render_soak_text(&report);

        assert!(text.contains("Daemon scenario soak"));
        assert!(text.contains("passed: true"));
        assert!(text.contains("planner_decisions:"));
        assert!(text.contains("cpu_millis_per_second:"));
    }
}
