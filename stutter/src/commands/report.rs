use std::path::PathBuf;

use crate::{process_tree::TaskClass, recommend, report, summary, validate};

pub fn run_summary_command(
    path: PathBuf,
    json: bool,
    top: usize,
    filter_class: Option<TaskClass>,
) -> anyhow::Result<()> {
    summary::summary_command(&path, json, top, filter_class)
}

pub fn run_validate_command(path: PathBuf, json: bool, strict: bool) -> anyhow::Result<()> {
    validate::validate_command(validate::ValidateCommandInput { path, json, strict })
}

#[allow(clippy::too_many_arguments)]
pub fn run_report_command(
    path: Option<PathBuf>,
    json: bool,
    analysis_json: bool,
    json_summary: bool,
    html: Option<PathBuf>,
    top: usize,
    cluster_window_ms: u64,
    batch: Option<PathBuf>,
    diff: Option<PathBuf>,
    filter_class: Option<TaskClass>,
    flamegraph_path: Option<PathBuf>,
) -> anyhow::Result<()> {
    if let Some(batch_dir) = batch {
        return report::print_batch_report(
            &batch_dir,
            diff.as_deref(),
            json_summary || json,
            top,
            filter_class,
        );
    }
    let Some(path) = path else {
        anyhow::bail!("report requires PATH unless --batch is set");
    };
    if let Some(diff_path) = diff {
        return report::print_diff_report(&diff_path, &path, top, filter_class);
    }
    if let Some(html_path) = html {
        report::write_html_report(&path, &html_path, top, cluster_window_ms, filter_class)?;
    }
    report::print_report(
        &path,
        json,
        analysis_json,
        json_summary,
        top,
        cluster_window_ms,
        filter_class,
        flamegraph_path,
    )
}

pub fn run_recommend_command(
    baseline: PathBuf,
    tune: PathBuf,
    json: bool,
    markdown: Option<PathBuf>,
) -> anyhow::Result<()> {
    recommend::recommend_command(recommend::RecommendCommandInput {
        baseline,
        tune,
        json,
        markdown,
    })
}

pub fn run_check_command(
    baseline: PathBuf,
    current: PathBuf,
    max_regression_p99_ms: Option<f64>,
    max_max_regression_ms: Option<f64>,
    json: bool,
    top: usize,
    filter_class: Option<TaskClass>,
) -> anyhow::Result<()> {
    report::check_regression(
        &baseline,
        &current,
        max_regression_p99_ms,
        max_max_regression_ms,
        json,
        top,
        filter_class,
    )
}
