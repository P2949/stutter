use crate::model::{
    DataQualitySummary, FrameDiagnosis, ReportHeaderSummary, ReportModel, SpikeCluster,
    TextReportCorrelationSections,
};

pub fn render_report(model: &ReportModel) -> String {
    let mut html = String::new();

    html.push_str("<!doctype html>\n");
    html.push_str("<html lang=\"en\">\n");
    html.push_str("<head>\n");
    html.push_str("  <meta charset=\"utf-8\">\n");
    html.push_str("  <title>stutter report</title>\n");
    html.push_str("</head>\n");
    html.push_str("<body>\n");
    html.push_str("  <main>\n");
    html.push_str("    <h1>stutter report</h1>\n");

    render_identity(&mut html, model);
    render_summary(&mut html, model);

    if let Some(header) = &model.header {
        render_header(&mut html, header);
    }

    if let Some(data_quality) = &model.data_quality {
        render_data_quality(&mut html, data_quality);
    }

    if !model.clusters.is_empty() {
        render_clusters(&mut html, &model.clusters);
    }

    if !model.frames.is_empty() {
        render_frames(&mut html, &model.frames);
    }

    if let Some(correlations) = &model.correlations
        && !correlations.sections.is_empty()
    {
        render_correlations(&mut html, correlations);
    }

    html.push_str("  </main>\n");
    html.push_str("</body>\n");
    html.push_str("</html>\n");

    html
}

fn render_identity(html: &mut String, model: &ReportModel) {
    html.push_str("    <section id=\"identity\">\n");
    html.push_str("      <h2>Identity</h2>\n");
    html.push_str("      <dl>\n");

    push_optional_dt_dd(html, "run_id", model.run_id().map(|run_id| run_id.as_str()));
    push_optional_dt_dd(
        html,
        "source_path",
        model.source_path().map(|path| path.as_str()),
    );

    if let Some(generated_at) = model.generated_at_unix_nanos() {
        push_dt_dd(
            html,
            "generated_at_unix_nanos",
            &generated_at.as_u128().to_string(),
        );
    }

    html.push_str("      </dl>\n");
    html.push_str("    </section>\n");
}

fn render_summary(html: &mut String, model: &ReportModel) {
    if model.score.is_none()
        && model.p95_latency_ns.is_none()
        && model.p99_latency_ns.is_none()
        && model.top_culprit.is_none()
    {
        return;
    }

    html.push_str("    <section id=\"summary\">\n");
    html.push_str("      <h2>Summary</h2>\n");
    html.push_str("      <dl>\n");

    if let Some(score) = model.score {
        push_dt_dd(html, "score", &format!("{score:.3}"));
    }
    if let Some(p95_latency_ns) = model.p95_latency_ns {
        push_dt_dd(html, "p95_latency_ns", &p95_latency_ns.to_string());
    }
    if let Some(p99_latency_ns) = model.p99_latency_ns {
        push_dt_dd(html, "p99_latency_ns", &p99_latency_ns.to_string());
    }
    push_optional_dt_dd(html, "top_culprit", model.top_culprit.as_deref());

    html.push_str("      </dl>\n");
    html.push_str("    </section>\n");
}

fn render_header(html: &mut String, header: &ReportHeaderSummary) {
    html.push_str("    <section id=\"header\">\n");
    html.push_str("      <h2>Run header</h2>\n");
    html.push_str("      <dl>\n");

    push_dt_dd(html, "file_path", &header.file_path);
    push_dt_dd(html, "schema_version", &header.schema_version.to_string());
    push_dt_dd(
        html,
        "expected_schema_version",
        &header.expected_schema_version.to_string(),
    );
    push_dt_dd(html, "run_name", &header.run_name);
    push_dt_dd(html, "duration_ms", &header.duration_ms.to_string());
    push_dt_dd(html, "stop_reason", &header.stop_reason);
    push_dt_dd(
        html,
        "active_target_pids_count",
        &header.active_target_pids_count.to_string(),
    );
    push_dt_dd(html, "watch_process", &header.watch_process);
    push_dt_dd(html, "persistent", &header.persistent.to_string());
    push_dt_dd(html, "csv_stream", &header.csv_stream);

    if !header.manual_pids.is_empty() {
        push_dt_dd(html, "manual_pids", &join_u32(&header.manual_pids));
    }
    if !header.tree_roots.is_empty() {
        push_dt_dd(html, "tree_roots", &join_u32(&header.tree_roots));
    }
    if !header.include_comm.is_empty() {
        push_dt_dd(html, "include_comm", &header.include_comm.join(", "));
    }
    if !header.exclude_comm.is_empty() {
        push_dt_dd(html, "exclude_comm", &header.exclude_comm.join(", "));
    }
    push_optional_dt_dd(
        html,
        "event_stream_warning",
        header.event_stream_warning.as_deref(),
    );

    html.push_str("      </dl>\n");
    html.push_str("    </section>\n");
}

fn render_data_quality(html: &mut String, data_quality: &DataQualitySummary) {
    html.push_str("    <section id=\"data-quality\">\n");
    html.push_str("      <h2>Data quality</h2>\n");
    html.push_str("      <dl>\n");

    push_dt_dd(html, "level", &format!("{:?}", data_quality.level));
    push_dt_dd(
        html,
        "schema_version",
        &data_quality.schema_version.to_string(),
    );
    push_dt_dd(
        html,
        "expected_schema_version",
        &data_quality.expected_schema_version.to_string(),
    );
    push_dt_dd(
        html,
        "event_stream_write_errors",
        &data_quality.event_stream_write_errors.to_string(),
    );
    push_dt_dd(
        html,
        "spike_events_truncated",
        &data_quality.spike_events_truncated.to_string(),
    );
    push_dt_dd(
        html,
        "drop_counters_nonzero",
        &data_quality.drop_counters_nonzero.to_string(),
    );
    push_dt_dd(
        html,
        "active_target_pids_count",
        &data_quality.active_target_pids_count.to_string(),
    );
    push_dt_dd(
        html,
        "block_io_correlation_confidence",
        &data_quality.block_io_correlation_confidence,
    );
    push_dt_dd(
        html,
        "frame_timestamp_alignment",
        &data_quality.frame_timestamp_alignment,
    );

    html.push_str("      </dl>\n");

    push_string_list(html, "Reasons", &data_quality.reasons);
    push_string_list(
        html,
        "Missing optional files",
        &data_quality.missing_optional_files,
    );
    push_string_list(html, "Validation errors", &data_quality.validation_errors);
    push_string_list(
        html,
        "Validation warnings",
        &data_quality.validation_warnings,
    );
    push_string_list(
        html,
        "Probe activation warnings",
        &data_quality.probe_activation_warnings,
    );

    html.push_str("    </section>\n");
}

fn render_clusters(html: &mut String, clusters: &[SpikeCluster]) {
    html.push_str("    <section id=\"spike-clusters\">\n");
    html.push_str("      <h2>Spike clusters</h2>\n");
    html.push_str("      <ol>\n");

    for cluster in clusters {
        html.push_str("        <li>\n");
        html.push_str("          <dl>\n");
        push_dt_dd(html, "distinct_tasks", &cluster.distinct_tasks.to_string());
        push_dt_dd(html, "point_count", &cluster.points.len().to_string());
        push_dt_dd(html, "min_switch_ns", &cluster.min_switch_ns.to_string());
        push_dt_dd(html, "max_switch_ns", &cluster.max_switch_ns.to_string());
        push_dt_dd(html, "max_latency_ns", &cluster.max_latency_ns.to_string());
        push_optional_dt_dd(
            html,
            "diagnosis",
            cluster.diagnosis.as_ref().map(|d| d.report_summary()),
        );
        html.push_str("          </dl>\n");

        if !cluster.points.is_empty() {
            html.push_str("          <table>\n");
            html.push_str(
                "            <thead><tr><th>task</th><th>comm</th><th>class</th><th>cpu</th><th>latency_ns</th></tr></thead>\n",
            );
            html.push_str("            <tbody>\n");
            for point in &cluster.points {
                html.push_str("              <tr>");
                push_td(html, &point.task.to_string());
                push_td(html, &point.comm);
                push_td(html, &point.class);
                push_td(html, &point.cpu.to_string());
                push_td(html, &point.latency_ns.to_string());
                html.push_str("</tr>\n");
            }
            html.push_str("            </tbody>\n");
            html.push_str("          </table>\n");
        }

        html.push_str("        </li>\n");
    }

    html.push_str("      </ol>\n");
    html.push_str("    </section>\n");
}

fn render_frames(html: &mut String, frames: &[FrameDiagnosis]) {
    html.push_str("    <section id=\"frame-diagnosis\">\n");
    html.push_str("      <h2>Frame diagnosis</h2>\n");
    html.push_str("      <ol>\n");

    for frame in frames {
        html.push_str("        <li>\n");
        html.push_str("          <dl>\n");
        push_dt_dd(
            html,
            "frame_elapsed_ms",
            &frame.frame_elapsed_ms.to_string(),
        );
        push_dt_dd(html, "frametime_ms", &frame.frametime_ms.to_string());
        push_dt_dd(html, "diagnosis", frame.diagnosis.report_summary());
        html.push_str("          </dl>\n");
        html.push_str("        </li>\n");
    }

    html.push_str("      </ol>\n");
    html.push_str("    </section>\n");
}

fn render_correlations(html: &mut String, correlations: &TextReportCorrelationSections) {
    html.push_str("    <section id=\"correlations\">\n");
    html.push_str("      <h2>Correlations</h2>\n");

    for section in &correlations.sections {
        html.push_str("      <section>\n");
        html.push_str("        <h3>");
        html.push_str(&escape_html(&section.title));
        html.push_str("</h3>\n");
        if section.lines.is_empty() {
            html.push_str("        <p>No correlation entries.</p>\n");
        } else {
            html.push_str("        <ul>\n");
            for line in &section.lines {
                html.push_str("          <li>");
                html.push_str(&escape_html(line));
                html.push_str("</li>\n");
            }
            html.push_str("        </ul>\n");
        }
        html.push_str("      </section>\n");
    }

    html.push_str("    </section>\n");
}

fn push_optional_dt_dd(html: &mut String, name: &str, value: Option<&str>) {
    if let Some(value) = value {
        push_dt_dd(html, name, value);
    }
}

fn push_dt_dd(html: &mut String, name: &str, value: &str) {
    html.push_str("        <dt>");
    html.push_str(&escape_html(name));
    html.push_str("</dt><dd>");
    html.push_str(&escape_html(value));
    html.push_str("</dd>\n");
}

fn push_td(html: &mut String, value: &str) {
    html.push_str("<td>");
    html.push_str(&escape_html(value));
    html.push_str("</td>");
}

fn push_string_list(html: &mut String, title: &str, values: &[String]) {
    if values.is_empty() {
        return;
    }

    html.push_str("      <h3>");
    html.push_str(&escape_html(title));
    html.push_str("</h3>\n");
    html.push_str("      <ul>\n");
    for value in values {
        html.push_str("        <li>");
        html.push_str(&escape_html(value));
        html.push_str("</li>\n");
    }
    html.push_str("      </ul>\n");
}

fn join_u32(values: &[u32]) -> String {
    values
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

fn escape_html(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());

    for ch in value.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(ch),
        }
    }

    escaped
}

#[cfg(test)]
mod tests {
    use stutter_core::{ids::RunId, paths::LogicalPath};

    use super::{escape_html, render_report};
    use crate::model::{
        DataQualityLevel, DataQualitySummary, Diagnosis, ReportHeaderSummary, ReportModel,
        SpikeCluster, SpikePoint, TextReportCorrelationSection, TextReportCorrelationSections,
    };

    fn minimal_data_quality(level: DataQualityLevel) -> DataQualitySummary {
        DataQualitySummary {
            level,
            reasons: vec!["low sample count <unsafe>".to_owned()],
            missing_optional_files: vec!["gpu.csv".to_owned()],
            validation_errors: Vec::new(),
            validation_warnings: vec!["schema drift & warning".to_owned()],
            probe_activation_warnings: Vec::new(),
            schema_version: 22,
            expected_schema_version: 22,
            event_stream_write_errors: 0,
            spike_events_truncated: false,
            spike_events_retained_count: 0,
            spike_events_dropped_count: 0,
            interval_record_count: 0,
            active_target_pids_count: 1,
            drop_counters_nonzero: false,
            percentile_scope_counts: Default::default(),
            block_io_correlation_basis: "none".to_owned(),
            block_io_correlation_confidence: "high".to_owned(),
            block_io_correlation_warning: None,
            frame_timestamp_alignment: "aligned".to_owned(),
            cpu_perf_requested: false,
            cpu_perf_open_errors: 0,
            cpu_perf_read_errors: 0,
            cpu_perf_skipped_tasks: 0,
        }
    }

    fn minimal_cluster() -> SpikeCluster {
        SpikeCluster {
            points: vec![SpikePoint {
                task: 42,
                class: "game".to_owned(),
                process_pid: None,
                comm: "render<&>".to_owned(),
                cpu: 3,
                wakeup_target_cpu: 3,
                latency_ns: 1_000_000,
                wakeup_ns: 1,
                switch_ns: 1_000_001,
                target_pending_wakeups: 0,
                observed_runnable_depth: 0,
                switch_prev_pid: 0,
                switch_prev_state: 1,
                switch_prev_state_label: "S".to_owned(),
                scx_ops: None,
                primary_cause: None,
                cause_tags: Vec::new(),
            }],
            distinct_tasks: 1,
            min_switch_ns: 1_000_001,
            max_switch_ns: 1_000_001,
            max_latency_ns: 1_000_000,
            diagnosis: Some(Diagnosis {
                primary: None,
                candidates: Vec::new(),
                missing_evidence: Vec::new(),
                candidate_rejections: Vec::new(),
                secondary_causes: Vec::new(),
                report_summary: "scheduler delay <high>".to_owned(),
            }),
            wake_graph: Vec::new(),
        }
    }

    #[test]
    fn escape_html_replaces_special_characters() {
        assert_eq!(
            escape_html("<tag attr=\"x\">Tom & Bob's</tag>"),
            "&lt;tag attr=&quot;x&quot;&gt;Tom &amp; Bob&#39;s&lt;/tag&gt;"
        );
    }

    #[test]
    fn html_report_renders_core_sections_and_escapes_content() {
        let mut model = ReportModel::new()
            .with_run_id(RunId::new("run-<001>"))
            .with_source_path(LogicalPath::new("runs/run-&-001"));

        model.score = Some(42.5);
        model.p95_latency_ns = Some(1_000_000);
        model.p99_latency_ns = Some(2_000_000);
        model.top_culprit = Some("render<&>".to_owned());
        model.header = Some(ReportHeaderSummary {
            file_path: "runs/run-<001>/summary.json".to_owned(),
            schema_version: 22,
            expected_schema_version: 22,
            run_name: "test run".to_owned(),
            duration_ms: 5000,
            stop_reason: "completed".to_owned(),
            manual_pids: vec![100],
            tree_roots: vec![200],
            include_comm: vec!["game".to_owned()],
            exclude_comm: vec!["browser".to_owned()],
            event_stream_warning: None,
            watch_process: "game".to_owned(),
            persistent: false,
            csv_stream: "none".to_owned(),
            active_target_pids_count: 1,
        });
        model.data_quality = Some(minimal_data_quality(DataQualityLevel::Low));
        model.clusters = vec![minimal_cluster()];
        model.correlations = Some(TextReportCorrelationSections {
            sections: vec![TextReportCorrelationSection {
                title: "IRQ & GPU".to_owned(),
                lines: vec!["irq wait < frame".to_owned()],
            }],
        });

        let html = render_report(&model);

        assert!(html.contains("<!doctype html>"));
        assert!(html.contains("<section id=\"identity\">"));
        assert!(html.contains("<section id=\"summary\">"));
        assert!(html.contains("<section id=\"header\">"));
        assert!(html.contains("<section id=\"data-quality\">"));
        assert!(html.contains("<section id=\"spike-clusters\">"));
        assert!(html.contains("<section id=\"correlations\">"));

        assert!(html.contains("run-&lt;001&gt;"));
        assert!(html.contains("runs/run-&amp;-001"));
        assert!(html.contains("render&lt;&amp;&gt;"));
        assert!(html.contains("low sample count &lt;unsafe&gt;"));
        assert!(html.contains("IRQ &amp; GPU"));
        assert!(!html.contains("render<&>"));
        assert!(!html.contains("low sample count <unsafe>"));
    }
}
