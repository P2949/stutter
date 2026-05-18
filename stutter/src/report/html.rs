use super::{analysis::*, text::*, *};

pub fn build_html_report_model(
    session: &SessionFile,
    artifacts: &session_io::RunArtifacts,
    analysis: &ReportAnalysisJson,
    top: usize,
    filter_class: Option<TaskClass>,
    legacy_text_report: Option<String>,
) -> anyhow::Result<HtmlReportModel> {
    let spike_events = if artifacts.spikes.is_empty() {
        None
    } else {
        Some(artifacts.spikes.clone())
    };

    let duration_ms = session.core.duration_ms.max(1);
    let bucket_ms = (duration_ms / 500).clamp(1, 1000);
    let spike_density = spike_events
        .as_deref()
        .map(|spikes| build_spike_density(spikes, bucket_ms))
        .unwrap_or_default();

    Ok(HtmlReportModel {
        session: session.clone(),
        data_quality: analysis.data_quality.clone(),
        cluster_analysis: analysis.cluster_analysis.clone(),
        frame_diagnoses: analysis.frame_diagnoses.clone(),
        frame_pacing: analysis.frame_pacing.clone(),
        pressure_timeline: analysis.pressure_timeline.clone(),
        runtime_slices: analysis.runtime_slices.clone(),
        artifacts_summary: analysis.artifacts_summary.clone(),
        focus_summary: analysis.focus_summary.clone(),
        foreground_summary: analysis.foreground_summary.clone(),
        top_tasks_by_max: top_task_rows_by_max_latency(session, top, filter_class),
        top_tasks_by_p99: top_task_rows_by_p99_latency(session, top, filter_class),
        spike_density,
        top_limit: top,
        spike_events,
        chart_artifacts: HtmlChartArtifacts {
            gpu_samples: artifacts.gpu_samples.clone(),
            frame_events: artifacts.frame_events.clone(),
        },
        legacy_text_report,
    })
}

pub fn render_html_report(model: &HtmlReportModel) -> anyhow::Result<String> {
    let model_json = escape_json_for_script_tag(
        &serde_json::to_string(model).context("failed to serialize HTML report model")?,
    );
    let session_json = escape_json_for_script_tag(
        &serde_json::to_string(&model.session).context("failed to serialize HTML session data")?,
    );
    let spike_events_json = escape_json_for_script_tag(
        &serde_json::to_string(&model.spike_events)
            .context("failed to serialize HTML spike event data")?,
    );
    let spike_density_json = escape_json_for_script_tag(
        &serde_json::to_string(&model.spike_density)
            .context("failed to serialize HTML spike density data")?,
    );
    let artifacts_json = escape_json_for_script_tag(
        &serde_json::to_string(&model.chart_artifacts)
            .context("failed to serialize HTML chart artifact data")?,
    );
    let cluster_analysis_json = escape_json_for_script_tag(
        &serde_json::to_string(&model.cluster_analysis)
            .context("failed to serialize HTML cluster data")?,
    );

    let template = include_str!("../report_template.html");

    Ok(template
        .replace("{html_report_model_json}", &model_json)
        .replace("{session_json}", &session_json)
        .replace("{spike_events_json}", &spike_events_json)
        .replace("{spike_density_json}", &spike_density_json)
        .replace("{artifacts_json}", &artifacts_json)
        .replace("{cluster_analysis_json}", &cluster_analysis_json)
        .replace("{top}", &model.top_limit.to_string()))
}

pub fn write_html_report(
    path: &Path,
    html_path: &Path,
    top: usize,
    cluster_window_ms: u64,
    filter_class: Option<TaskClass>,
) -> anyhow::Result<()> {
    let input = load_report_input(path)?;
    let ReportBuildResult {
        analysis,
        artifacts,
    } = build_report_analysis_from_input(input, top, cluster_window_ms, filter_class)?;

    let text_report = render_report(
        path,
        &analysis.session,
        &analysis.cluster_analysis,
        &analysis.frame_diagnoses,
        &artifacts,
        &analysis.focus_summary,
        &analysis.foreground_summary,
        top,
        cluster_window_ms,
        filter_class,
    );
    let model = build_html_report_model(
        &analysis.session,
        &artifacts,
        &analysis,
        top,
        filter_class,
        Some(text_report),
    )?;
    let html = render_html_report(&model)?;
    if let Some(parent) = html_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(html_path, html)
        .with_context(|| format!("failed to write HTML report {}", html_path.display()))?;
    Ok(())
}

pub(crate) fn task_html_row(task: &SessionTask) -> TaskHtmlRow {
    TaskHtmlRow {
        task: task.task,
        active: task.active,
        class: task.class,
        process_pid: task.process_pid,
        process_comm: task.process_comm.to_string(),
        comm: task.comm.clone(),
        samples: task.latency.samples,
        spike_count: task.latency.over_1ms,
        max_latency_ms: ns_to_ms(task.latency.max_ns),
        p99_latency_ms: ns_to_ms(task.latency.p99_ns),
        avg_latency_ms: ns_to_ms(task.latency.avg_ns),
        over_1ms: task.latency.over_1ms,
        over_2ms: task.latency.over_2ms,
        over_5ms: task.latency.over_5ms,
    }
}
