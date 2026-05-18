use super::model::RunDiffSummary;
use crate::{metrics::format_latency, summary::format_latency_signed};

pub(crate) fn render_run_diff_summary(diff: &RunDiffSummary, top: usize) -> String {
    let mut output = String::new();
    pushln(&mut output, "stutter diff report");
    pushln(&mut output, "===================");
    pushln(
        &mut output,
        format!(
            "run_a: {} ({}ms)",
            diff.baseline_run_name.as_deref().unwrap_or("-"),
            diff.baseline_duration_ms,
        ),
    );
    pushln(
        &mut output,
        format!(
            "run_b: {} ({}ms)",
            diff.current_run_name.as_deref().unwrap_or("-"),
            diff.current_duration_ms,
        ),
    );
    pushln(&mut output, "");

    pushln(&mut output, "summary highlights");
    pushln(&mut output, "------------------");
    if let Some(worst) = &diff.worst_max_regression {
        let pct = if worst.baseline_max_ns > 0 {
            format!(
                " (+{:.1}%)",
                (worst.delta_max_ns as f64 / worst.baseline_max_ns as f64) * 100.0
            )
        } else {
            String::new()
        };
        pushln(
            &mut output,
            format!(
                "biggest regression:  {} on comm={} process={}{}",
                format_latency_signed(worst.delta_max_ns),
                worst.identity.comm,
                worst.identity.process_comm,
                pct
            ),
        );
    }
    if let Some(best) = diff.improvements.first() {
        let pct = if best.baseline_max_ns > 0 {
            format!(
                " ({:.1}%)",
                (best.delta_max_ns as f64 / best.baseline_max_ns as f64) * 100.0
            )
        } else {
            String::new()
        };
        pushln(
            &mut output,
            format!(
                "biggest improvement: {} on comm={} process={}{}",
                format_latency_signed(best.delta_max_ns),
                best.identity.comm,
                best.identity.process_comm,
                pct
            ),
        );
    }
    pushln(&mut output, "");

    pushln(&mut output, "regressions (worse in run_b)");
    pushln(&mut output, "---------------------------");
    if diff.regressions.is_empty() {
        pushln(&mut output, "none");
    }
    for d in diff.regressions.iter().take(top) {
        pushln(
            &mut output,
            format!(
                "class={:?} comm={} process={} max: {} -> {} (delta={}) p99_delta={} over_1ms_delta={}",
                d.identity.class,
                d.identity.comm,
                d.identity.process_comm,
                format_latency(d.baseline_max_ns),
                format_latency(d.current_max_ns),
                format_latency_signed(d.delta_max_ns),
                format_latency_signed(d.delta_p99_ns),
                if d.delta_over_1ms >= 0 {
                    format!("+{}", d.delta_over_1ms)
                } else {
                    d.delta_over_1ms.to_string()
                },
            ),
        );
    }
    pushln(&mut output, "");

    pushln(&mut output, "improvements (better in run_b)");
    pushln(&mut output, "-----------------------------");
    if diff.improvements.is_empty() {
        pushln(&mut output, "none");
    }
    for d in diff.improvements.iter().take(top) {
        pushln(
            &mut output,
            format!(
                "class={:?} comm={} process={} max: {} -> {} (delta={}) p99_delta={} over_1ms_delta={}",
                d.identity.class,
                d.identity.comm,
                d.identity.process_comm,
                format_latency(d.baseline_max_ns),
                format_latency(d.current_max_ns),
                format_latency_signed(d.delta_max_ns),
                format_latency_signed(d.delta_p99_ns),
                if d.delta_over_1ms >= 0 {
                    format!("+{}", d.delta_over_1ms)
                } else {
                    d.delta_over_1ms.to_string()
                },
            ),
        );
    }
    pushln(&mut output, "");

    if !diff.new_tasks.is_empty() {
        pushln(&mut output, "new tasks (only in run_b)");
        pushln(&mut output, "------------------------");
        for task in diff.new_tasks.iter().take(top) {
            pushln(
                &mut output,
                format!(
                    "comm={} process={} class={:?}",
                    task.identity.comm, task.identity.process_comm, task.identity.class
                ),
            );
        }
        pushln(&mut output, "");
    }

    if !diff.removed_tasks.is_empty() {
        pushln(&mut output, "removed tasks (only in run_a)");
        pushln(&mut output, "----------------------------");
        for task in diff.removed_tasks.iter().take(top) {
            pushln(
                &mut output,
                format!(
                    "comm={} process={} class={:?}",
                    task.identity.comm, task.identity.process_comm, task.identity.class
                ),
            );
        }
        pushln(&mut output, "");
    }

    output
}

fn pushln(output: &mut String, line: impl AsRef<str>) {
    output.push_str(line.as_ref());
    output.push('\n');
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::render_run_diff_summary;
    use crate::report::diff::RunDiffSummary;

    #[test]
    fn diff_renderer_accepts_prebuilt_diff_model() {
        let diff = RunDiffSummary {
            baseline_path: PathBuf::from("run-a"),
            current_path: PathBuf::from("run-b"),
            baseline_run_name: Some("run-a".to_owned()),
            current_run_name: Some("run-b".to_owned()),
            baseline_duration_ms: 1000,
            current_duration_ms: 2000,
            filter_class: None,
            compared_tasks: 0,
            worst_p99_regression: None,
            worst_max_regression: None,
            regressions: Vec::new(),
            improvements: Vec::new(),
            new_scored_tasks: Vec::new(),
            new_tasks: Vec::new(),
            removed_tasks: Vec::new(),
        };

        let rendered = render_run_diff_summary(&diff, 10);

        assert!(rendered.contains("stutter diff report"));
        assert!(rendered.contains("run_a: run-a (1000ms)"));
        assert!(rendered.contains("run_b: run-b (2000ms)"));
    }
}
