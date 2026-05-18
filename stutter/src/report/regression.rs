use super::*;

#[derive(Clone, Debug, Serialize)]
pub struct RegressionCheckSummary {
    pub passed: bool,
    pub baseline_path: PathBuf,
    pub current_path: PathBuf,
    pub max_regression_p99_ms: Option<f64>,
    pub max_max_regression_ms: Option<f64>,
    pub violations: Vec<RegressionViolation>,
    pub diff: RunDiffSummary,
}

#[derive(Clone, Debug, Serialize)]
pub struct RegressionViolation {
    pub metric: RegressionMetric,
    pub comm: String,
    pub process_comm: String,
    pub class: TaskClass,
    pub delta_ns: i64,
    pub threshold_ns: i64,
    pub new_task: bool,
}

pub fn check_regression(
    path_baseline: &Path,
    path_current: &Path,
    max_regression_p99_ms: Option<f64>,
    max_max_regression_ms: Option<f64>,
    json: bool,
    top: usize,
    filter_class: Option<TaskClass>,
) -> anyhow::Result<()> {
    let diff = build_run_diff_summary(path_baseline, path_current, filter_class)?;
    let mut violations = Vec::new();

    if let Some(threshold_ms) = max_regression_p99_ms {
        let threshold_ns = ms_to_ns_i64(threshold_ms);
        for delta in diff
            .regressions
            .iter()
            .filter(|delta| delta.delta_p99_ns > threshold_ns)
        {
            violations.push(violation_from_delta(
                RegressionMetric::P99,
                delta,
                delta.delta_p99_ns,
                threshold_ns,
            ));
        }
        for task in diff
            .new_scored_tasks
            .iter()
            .filter(|task| task.p99_ns as i64 > threshold_ns)
        {
            violations.push(RegressionViolation {
                metric: RegressionMetric::P99,
                comm: task.identity.comm.clone(),
                process_comm: task.identity.process_comm.clone(),
                class: task.identity.class,
                delta_ns: task.p99_ns as i64,
                threshold_ns,
                new_task: true,
            });
        }
    }

    if let Some(threshold_ms) = max_max_regression_ms {
        let threshold_ns = ms_to_ns_i64(threshold_ms);
        for delta in diff
            .regressions
            .iter()
            .filter(|delta| delta.delta_max_ns > threshold_ns)
        {
            violations.push(violation_from_delta(
                RegressionMetric::Max,
                delta,
                delta.delta_max_ns,
                threshold_ns,
            ));
        }
        for task in diff
            .new_scored_tasks
            .iter()
            .filter(|task| task.max_ns as i64 > threshold_ns)
        {
            violations.push(RegressionViolation {
                metric: RegressionMetric::Max,
                comm: task.identity.comm.clone(),
                process_comm: task.identity.process_comm.clone(),
                class: task.identity.class,
                delta_ns: task.max_ns as i64,
                threshold_ns,
                new_task: true,
            });
        }
    }

    violations.sort_by_key(|violation| std::cmp::Reverse(violation.delta_ns));
    let passed = violations.is_empty();
    let output = RegressionCheckSummary {
        passed,
        baseline_path: path_baseline.to_path_buf(),
        current_path: path_current.to_path_buf(),
        max_regression_p99_ms,
        max_max_regression_ms,
        violations,
        diff: diff.limited(top),
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        print!("{}", render_check_summary(&output, top));
    }

    if !output.passed {
        if let Some(first) = output.violations.first() {
            anyhow::bail!(
                "regression_check_failed metric={:?} regressed_by={} comm={} process={} max_allowed={}",
                first.metric,
                format_latency_signed(first.delta_ns),
                first.comm,
                first.process_comm,
                format_latency(first.threshold_ns as u64)
            );
        }
        anyhow::bail!("regression_check_failed");
    }

    Ok(())
}

pub fn check_percentile_regression(
    path_baseline: &Path,
    path_current: &Path,
    max_regression_p99_ms: f64,
) -> anyhow::Result<()> {
    check_regression(
        path_baseline,
        path_current,
        Some(max_regression_p99_ms),
        None,
        false,
        10,
        None,
    )
}
