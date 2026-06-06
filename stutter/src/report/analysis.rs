use super::*;

mod clusters;
mod correlation;
mod density;
mod diagnosis;
mod display_path;
mod foreground;
mod format;
mod frame;
mod pressure;
mod quality;
mod runtime;
mod tasks;
mod timing;

pub(crate) use clusters::*;
pub(crate) use correlation::text_report_correlation_sections;
pub use density::build_spike_density;
pub(crate) use density::{median_f64, ms_to_ns_i64, ns_to_ms, percentile_f64};
pub(crate) use diagnosis::*;
pub(crate) use display_path::*;
#[cfg(test)]
pub(crate) use foreground::foreground_for_cluster;
pub(crate) use foreground::{
    annotate_clusters_with_foreground, focus_report_summary, foreground_for_elapsed_ms,
    foreground_report_summary,
};
pub(crate) use format::*;
pub(crate) use frame::*;
pub(crate) use pressure::*;
pub(crate) use quality::*;
pub(crate) use runtime::*;
pub(crate) use tasks::*;
pub(crate) use timing::{
    build_cross_gpu_fence_summary, build_direct_scanout_summary, build_dmabuf_path_summary,
    build_drm_fence_timing_summary, build_gpu_engine_activity_summary, build_kms_timing_summary,
    build_wayland_presentation_summary,
};

pub fn build_report_analysis(
    path: &Path,
    top: usize,
    cluster_window_ms: u64,
    filter_class: Option<TaskClass>,
) -> anyhow::Result<ReportAnalysisJson> {
    let input = load_report_input(path)?;
    Ok(build_report_analysis_from_input(input, top, cluster_window_ms, filter_class)?.analysis)
}

pub(crate) fn build_report_analysis_from_input(
    input: ReportInputModel,
    top: usize,
    cluster_window_ms: u64,
    filter_class: Option<TaskClass>,
) -> anyhow::Result<ReportBuildResult> {
    let mut artifacts = input.into_artifacts();
    let session = artifacts.session.clone();

    let median_frametime = calculate_median_frametime(&artifacts.frame_events);
    let frame_spikes = identify_frame_spikes(&artifacts.frame_events, median_frametime);

    let cluster_window_ns = cluster_window_ms.saturating_mul(1_000_000);
    let spike_events_ref = if !artifacts.spikes.is_empty() {
        Some(&artifacts.spikes[..])
    } else {
        None
    };

    let cluster_analysis = spike_cluster_analysis(
        &session,
        spike_events_ref,
        cluster_window_ns,
        top,
        filter_class,
    );

    let windows = compute_correlation_windows(
        &session,
        &cluster_analysis.clusters,
        &frame_spikes,
        cluster_window_ns,
    );
    artifacts.load_correlations(windows)?;

    let mut cluster_analysis = cluster_analysis;
    perform_diagnosis(
        &mut cluster_analysis.clusters,
        &artifacts,
        cluster_window_ns,
    );

    annotate_clusters_with_foreground(
        &mut cluster_analysis.clusters,
        &artifacts.foreground_events,
        session.config.foreground_max_stale_ms.max(2_500),
    );

    let all_spike_points = if !artifacts.spikes.is_empty() {
        flatten_spike_events(&session, &artifacts.spikes)
    } else {
        flatten_top_spikes(&session)
    };

    let frame_diagnoses = perform_frame_diagnosis(
        &session,
        &frame_spikes,
        &all_spike_points,
        &artifacts,
        cluster_window_ns,
    );
    let frame_pacing = build_frame_pacing_summary(
        &artifacts.frame_events,
        &cluster_analysis.clusters,
        &artifacts.foreground_events,
        session.config.foreground_max_stale_ms.max(2_500),
    );

    let data_quality = data_quality_summary(&session, &artifacts.validation);
    let artifacts_summary = artifacts_summary_from_session(&session);
    let pressure_timeline = build_pressure_timeline(
        &artifacts.intervals,
        &cluster_analysis.clusters,
        cluster_window_ms,
    );
    let runtime_slices = runtime_slice_analysis_summary(&session, &artifacts);

    let focus_summary = focus_report_summary(&session, &artifacts.focus_events);
    let foreground_summary = foreground_report_summary(&session, &artifacts.foreground_events);
    let kms_timing = build_kms_timing_summary(&artifacts.kms_flip_events);
    let drm_fence_timing = build_drm_fence_timing_summary(
        &artifacts.drm_fence_events,
        &artifacts.kms_flip_events,
        &artifacts.frame_events,
    );
    let cross_gpu_fence = build_cross_gpu_fence_summary(
        &artifacts.drm_fence_events,
        &artifacts.kms_flip_events,
        &artifacts.frame_events,
        artifacts.display_topology.as_ref(),
    );
    let wayland_presentation = build_wayland_presentation_summary(
        &artifacts.wayland_presentation_events,
        &artifacts.kms_flip_events,
        &artifacts.frame_events,
    );
    let direct_scanout = build_direct_scanout_summary(
        &artifacts.wayland_presentation_events,
        artifacts.display_topology.as_ref(),
    );
    let dmabuf_path = build_dmabuf_path_summary(&artifacts.dmabuf_events);
    let gpu_engine_activity =
        build_gpu_engine_activity_summary(&artifacts.gpu_engine_samples, &artifacts.frame_events);
    let display_path_diagnosis = build_display_path_diagnosis_summary(
        &session,
        DisplayPathDiagnosisInputs {
            frame_pacing: &frame_pacing,
            kms_timing: &kms_timing,
            drm_fence_timing: &drm_fence_timing,
            cross_gpu_fence: &cross_gpu_fence,
            wayland_presentation: &wayland_presentation,
            direct_scanout: &direct_scanout,
            dmabuf_path: &dmabuf_path,
            gpu_engine_activity: &gpu_engine_activity,
        },
    );
    let diagnosis_thresholds = crate::diagnosis::DiagnosisConfig::default().threshold_table();

    Ok(ReportBuildResult {
        analysis: ReportAnalysisJson {
            session,
            cluster_analysis,
            frame_diagnoses,
            frame_pacing,
            pressure_timeline,
            runtime_slices,
            diagnosis_thresholds,
            artifacts_summary,
            data_quality,
            focus_summary,
            foreground_summary,
            kms_timing,
            drm_fence_timing,
            cross_gpu_fence,
            wayland_presentation,
            direct_scanout,
            dmabuf_path,
            gpu_engine_activity,
            display_path_diagnosis,
        },
        artifacts,
    })
}

pub(crate) fn event_stream_warning(
    event_stream_write_errors: u64,
    first_event_stream_write_error: Option<&str>,
) -> Option<String> {
    if event_stream_write_errors == 0 {
        return None;
    }

    let first = first_event_stream_write_error
        .filter(|s| !s.is_empty())
        .unwrap_or("first error was not recorded");

    Some(format!(
        "WARNING: recording event streams had {event_stream_write_errors} write error(s); \
         event artifact files may be incomplete. First error: {first}"
    ))
}

pub(crate) fn artifacts_summary_from_session(session: &SessionFile) -> ArtifactsSummary {
    ArtifactsSummary {
        spike_count: session
            .core
            .spike_events_retained_count
            .max(session.top_spikes.len() as u64),
        frame_count: session.core.frame_event_count,
        irq_event_count: session.core.irq_event_count,
        gpu_sample_count: session.core.gpu_sample_count,
        frame_event_count: session.core.frame_event_count,
        migration_event_count: session.core.migration_event_count.unwrap_or(0),
        cpu_freq_sample_count: session.core.cpu_freq_sample_count.unwrap_or(0),
        block_io_event_count: session.core.block_io_event_count,
        runtime_slice_count: session.core.runtime_slice_count,
        interval_record_count: session.core.interval_record_count,
        scx_event_count: session.core.scx_event_count,
        focus_event_count: session.core.focus_event_count,
        foreground_event_count: session.core.foreground_event_count,
        kms_flip_event_count: session.core.kms_flip_event_count,
        drm_fence_event_count: session.core.drm_fence_event_count,
        wayland_presentation_event_count: session.core.wayland_presentation_event_count,
        dmabuf_event_count: session.core.dmabuf_event_count,
        gpu_engine_sample_count: session.core.gpu_engine_sample_count,
    }
}

pub(crate) fn violation_from_delta(
    metric: RegressionMetric,
    delta: &TaskDeltaSummary,
    delta_ns: i64,
    threshold_ns: i64,
) -> RegressionViolation {
    RegressionViolation {
        metric,
        comm: delta.identity.comm.clone(),
        process_comm: delta.identity.process_comm.clone(),
        class: delta.identity.class,
        delta_ns,
        threshold_ns,
        new_task: false,
    }
}
