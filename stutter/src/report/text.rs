use super::{
    analysis::{block_io_correlation_basis, text_report_correlation_sections},
    build_report_analysis_from_input,
    render::{json::render_json_pretty, text::render_report},
    *,
};

#[allow(clippy::too_many_arguments)]
pub fn print_report(
    path: &Path,
    json: bool,
    analysis_json: bool,
    json_summary: bool,
    top: usize,
    cluster_window_ms: u64,
    filter_class: Option<TaskClass>,
    flamegraph: Option<PathBuf>,
) -> anyhow::Result<()> {
    if json {
        let session = load_report_session(path)?;
        println!("{}", render_json_pretty(&session)?);
        return Ok(());
    }

    if json_summary {
        let summary = summary::build_compact_run_summary(path, top, filter_class)?;
        println!("{}", render_json_pretty(&summary)?);
        return Ok(());
    }

    if analysis_json {
        let input = load_report_input(path)?;
        let analysis =
            build_report_analysis_from_input(input, top, cluster_window_ms, filter_class)?.analysis;
        println!("{}", render_json_pretty(&analysis)?);
        return Ok(());
    }

    let input = load_report_input(path)?;
    let ReportBuildResult {
        analysis,
        artifacts,
    } = build_report_analysis_from_input(input, top, cluster_window_ms, filter_class)?;
    let correlation_sections = text_report_correlation_sections(
        &analysis.cluster_analysis.clusters,
        &artifacts,
        block_io_correlation_basis(&analysis.session),
        cluster_window_ms.saturating_mul(1_000_000),
        top,
    );

    if let Some(flamegraph_path) = flamegraph {
        crate::flamegraph::write_latency_flamegraph_svg(&artifacts.spikes, &flamegraph_path)?;
    }

    print!(
        "{}",
        render_report(
            path,
            &analysis.session,
            &analysis.cluster_analysis,
            &analysis.frame_diagnoses,
            &analysis.data_quality,
            &analysis.pressure_timeline,
            &analysis.runtime_slices,
            &correlation_sections,
            &analysis.focus_summary,
            &analysis.foreground_summary,
            top,
            cluster_window_ms,
            filter_class,
        )
    );

    Ok(())
}
