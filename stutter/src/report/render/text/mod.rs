use std::path::Path;

use super::super::{
    MIN_CLUSTER_TASKS,
    analysis::{
        block_io_correlation_basis, block_io_correlation_confidence, block_io_correlation_warning,
        event_stream_warning, format_optional_ratio, format_pressure_option, format_task_cpu_perf,
        percentile_warning_note,
    },
    *,
};
use crate::{
    metrics::format_latency, recorder::SESSION_SCHEMA_VERSION,
    sched_state::classify_switch_prev_state,
};

mod cluster;
mod correlation;
mod diagnosis;
mod frame;
mod header;
mod pressure;
mod runtime;
mod summary;

#[cfg(test)]
pub(crate) use cluster::map_cluster;
pub(crate) use diagnosis::render_display_path_diagnosis_text;
pub(crate) use pressure::render_pressure_timeline_summary;
pub(crate) use runtime::render_runtime_slice_summary;
pub(crate) use summary::{
    render_check_summary, render_focus_summary_text, render_foreground_summary_text,
};

pub(crate) struct TextReportRenderInput<'a> {
    pub path: &'a Path,
    pub session: &'a SessionFile,
    pub cluster_analysis: &'a SpikeClusterAnalysis,
    pub frame_diagnoses: &'a [FrameDiagnosis],
    pub data_quality: &'a DataQualitySummary,
    pub pressure_timeline: &'a PressureTimelineSummary,
    pub runtime_slice_summary: &'a RuntimeSliceAnalysisSummary,
    pub correlation_sections: &'a TextReportCorrelationSections,
    pub focus_summary: &'a FocusReportSummary,
    pub foreground_summary: &'a ForegroundReportSummary,
    pub display_path_diagnosis: Option<&'a DisplayPathDiagnosisSummary>,
    pub top: usize,
    pub cluster_window_ms: u64,
    pub filter_class: Option<TaskClass>,
}

pub(crate) fn render_report(input: TextReportRenderInput<'_>) -> String {
    let mut writer = ReportTextWriter::new();

    writer.line("stutter report");
    writer.line("==============");

    writer.append(&render_focus_summary_text(input.focus_summary));
    writer.append(&render_foreground_summary_text(input.foreground_summary));
    if let Some(display_path_diagnosis) = input.display_path_diagnosis {
        writer.append(&render_display_path_diagnosis_text(display_path_diagnosis));
    }

    writer.append(&header::render_header_section(input.path, input.session));
    writer.append(&header::render_data_quality_section(input.data_quality));

    if pressure::pressure_timeline_has_pressure(input.pressure_timeline) {
        writer.append(&render_pressure_timeline_summary(input.pressure_timeline));
        writer.blank();
    }

    writer.append(&render_runtime_slice_summary(
        input.runtime_slice_summary,
        input.top,
    ));
    writer.append(&correlation::render_pre_task_warning_sections(
        input.session,
        input.top,
    ));
    writer.append(&cluster::render_task_latency_sections(
        input.session,
        input.top,
        input.filter_class,
    ));
    writer.append(&cluster::render_top_spikes(input.session, input.top));
    writer.append(&cluster::render_spike_clusters(
        input.cluster_analysis,
        input.top,
        input.cluster_window_ms,
    ));
    writer.append(&frame::render_frame_diagnoses(
        input.frame_diagnoses,
        input.top,
    ));
    writer.append(&correlation::render_correlation_sections(
        input.correlation_sections,
    ));

    writer.finish()
}

pub(crate) struct ReportTextWriter {
    lines: Vec<String>,
    section_depth: usize,
}

impl ReportTextWriter {
    pub(crate) fn new() -> Self {
        Self {
            lines: Vec::new(),
            section_depth: 0,
        }
    }

    pub(crate) fn line(&mut self, line: impl AsRef<str>) {
        let line = line.as_ref();
        if self.section_depth == 0 || line.is_empty() {
            self.lines.push(line.to_owned());
        } else {
            self.lines
                .push(format!("{}{}", "  ".repeat(self.section_depth), line));
        }
    }

    pub(crate) fn blank(&mut self) {
        self.line("");
    }

    pub(crate) fn append(&mut self, text: &str) {
        for line in text.split_inclusive('\n') {
            if let Some(stripped) = line.strip_suffix('\n') {
                self.lines.push(stripped.to_owned());
            } else if !line.is_empty() {
                self.lines.push(line.to_owned());
            }
        }
    }

    pub(crate) fn finish(self) -> String {
        if self.lines.is_empty() {
            return String::new();
        }

        let mut output = self.lines.join("\n");
        output.push('\n');
        output
    }
}
