use std::time::Duration;

use crate::daemon::{DaemonOverheadMonitor, DaemonOverheadReport};

pub fn run_bench_overhead_command(
    input: crate::commands::input::DaemonBenchOverheadCommandInput,
) -> anyhow::Result<()> {
    let report = DaemonOverheadMonitor::default()
        .sample_over_duration(Duration::from_millis(input.duration_ms));

    if input.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", render_bench_overhead_text(&report));
    }

    Ok(())
}

pub fn render_bench_overhead_text(report: &DaemonOverheadReport) -> String {
    let mut text = String::new();

    text.push_str("Daemon overhead benchmark\n");
    text.push_str("=========================\n");
    text.push_str(&format!("within_budget: {}\n", report.within_budget));
    text.push_str(&format!(
        "sample_duration_millis: {}\n",
        report.snapshot.sample_duration_millis
    ));
    text.push_str(&format!(
        "cpu_millis_per_second: {} / {}\n",
        report.snapshot.cpu_millis_per_second, report.budget.max_cpu_millis_per_second
    ));
    if let Some(bytes) = report.snapshot.memory_rss_bytes {
        text.push_str(&format!(
            "memory_rss_bytes: {} / {}\n",
            bytes, report.budget.max_memory_bytes
        ));
    }
    if let Some(fds) = report.snapshot.open_fds {
        text.push_str(&format!(
            "open_fds: {} / {}\n",
            fds, report.budget.max_open_fds
        ));
    }
    if let Some(bytes) = report.snapshot.disk_write_bytes_per_minute {
        text.push_str(&format!(
            "disk_write_bytes_per_minute: {} / {}\n",
            bytes, report.budget.max_disk_write_bytes_per_minute
        ));
    }
    for issue in &report.issues {
        text.push_str(&format!(
            "issue: {} - {}\n",
            issue.reason_code, issue.message
        ));
    }

    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_bench_overhead_text_contains_budget_status() {
        let report = crate::daemon::DaemonOverheadMonitor::default()
            .sample_over_duration(Duration::from_millis(10));

        let text = render_bench_overhead_text(&report);

        assert!(text.contains("Daemon overhead benchmark"));
        assert!(text.contains("within_budget:"));
        assert!(text.contains("cpu_millis_per_second:"));
    }
}
