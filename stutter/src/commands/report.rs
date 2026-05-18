use crate::{
    commands::input::{
        CheckCommandInput, RecommendCommandInput, ReportCommandInput, SummaryCommandInput,
        ValidateCommandInput,
    },
    recommend, report, summary, validate,
};

pub fn run_summary_command(input: SummaryCommandInput) -> anyhow::Result<()> {
    summary::summary_command(&input.path, input.json, input.top, input.filter_class)
}

pub fn run_validate_command(input: ValidateCommandInput) -> anyhow::Result<()> {
    validate::validate_command(validate::ValidateCommandInput {
        path: input.path,
        json: input.json,
        strict: input.strict,
    })
}

pub fn run_report_command(input: ReportCommandInput) -> anyhow::Result<()> {
    if let Some(batch_dir) = input.batch {
        return report::print_batch_report(
            &batch_dir,
            input.diff.as_deref(),
            input.json_summary || input.json,
            input.top,
            input.filter_class,
        );
    }
    let Some(path) = input.path else {
        anyhow::bail!("report requires PATH unless --batch is set");
    };
    if let Some(diff_path) = input.diff {
        return report::print_diff_report(&diff_path, &path, input.top, input.filter_class);
    }
    if let Some(html_path) = input.html {
        report::write_html_report(
            &path,
            &html_path,
            input.top,
            input.cluster_window_ms,
            input.filter_class,
        )?;
    }
    report::print_report(report::PrintReportInput {
        path: &path,
        json: input.json,
        analysis_json: input.analysis_json,
        json_summary: input.json_summary,
        top: input.top,
        cluster_window_ms: input.cluster_window_ms,
        filter_class: input.filter_class,
        flamegraph: input.flamegraph,
    })
}

pub fn run_recommend_command(input: RecommendCommandInput) -> anyhow::Result<()> {
    recommend::recommend_command(recommend::RecommendCommandInput {
        baseline: input.baseline,
        tune: input.tune,
        json: input.json,
        markdown: input.markdown,
    })
}

pub fn run_check_command(input: CheckCommandInput) -> anyhow::Result<()> {
    report::check_regression(
        &input.baseline,
        &input.current,
        input.max_regression_p99_ms,
        input.max_max_regression_ms,
        input.json,
        input.top,
        input.filter_class,
    )
}
