use super::{
    analysis::{block_io_correlation_basis, text_report_correlation_sections},
    build_report_analysis_from_input,
    render::{
        json::render_json_pretty,
        text::{TextReportRenderInput, render_report},
    },
    *,
};

pub struct PrintReportInput<'a> {
    pub path: &'a Path,
    pub json: bool,
    pub analysis_json: bool,
    pub json_summary: bool,
    pub top: usize,
    pub cluster_window_ms: u64,
    pub filter_class: Option<TaskClass>,
    pub flamegraph: Option<PathBuf>,
}

pub fn print_report(input: PrintReportInput<'_>) -> anyhow::Result<()> {
    if input.json {
        let session = load_report_session(input.path)?;
        println!("{}", render_json_pretty(&session)?);
        return Ok(());
    }

    if input.json_summary {
        let summary =
            summary::build_compact_run_summary(input.path, input.top, input.filter_class)?;
        println!("{}", render_json_pretty(&summary)?);
        return Ok(());
    }

    if input.analysis_json {
        let report_input = load_report_input(input.path)?;
        let analysis = build_report_analysis_from_input(
            report_input,
            input.top,
            input.cluster_window_ms,
            input.filter_class,
        )?
        .analysis;
        println!("{}", render_json_pretty(&analysis)?);
        return Ok(());
    }

    let report_input = load_report_input(input.path)?;
    let ReportBuildResult {
        analysis,
        artifacts,
    } = build_report_analysis_from_input(
        report_input,
        input.top,
        input.cluster_window_ms,
        input.filter_class,
    )?;
    let correlation_sections = text_report_correlation_sections(
        &analysis.cluster_analysis.clusters,
        &artifacts,
        block_io_correlation_basis(&analysis.session),
        input.cluster_window_ms.saturating_mul(1_000_000),
        input.top,
    );

    if let Some(flamegraph_path) = input.flamegraph {
        crate::flamegraph::write_latency_flamegraph_svg(&artifacts.spikes, &flamegraph_path)?;
    }

    print!(
        "{}",
        render_report(TextReportRenderInput {
            path: input.path,
            session: &analysis.session,
            cluster_analysis: &analysis.cluster_analysis,
            frame_diagnoses: &analysis.frame_diagnoses,
            data_quality: &analysis.data_quality,
            pressure_timeline: &analysis.pressure_timeline,
            runtime_slice_summary: &analysis.runtime_slices,
            correlation_sections: &correlation_sections,
            focus_summary: &analysis.focus_summary,
            foreground_summary: &analysis.foreground_summary,
            top: input.top,
            cluster_window_ms: input.cluster_window_ms,
            filter_class: input.filter_class,
        })
    );

    Ok(())
}
