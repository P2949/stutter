//! Frame-pacing outlier analysis and report correlation-window helpers.

use std::collections::BTreeSet;

use super::*;

pub(crate) fn calculate_median_frametime(frames: &[FrameEvent]) -> f64 {
    if frames.is_empty() {
        return 0.0;
    }
    let mut times: Vec<_> = frames.iter().map(|f| f.frametime_ms).collect();
    times.sort_by(|a, b| a.total_cmp(b));
    let mid = times.len() / 2;
    if times.len() % 2 == 0 {
        (times[mid - 1] + times[mid]) / 2.0
    } else {
        times[mid]
    }
}

pub(crate) fn identify_frame_spikes(frames: &[FrameEvent], median: f64) -> Vec<FrameEvent> {
    let threshold = if median.is_finite() && median > 0.0 {
        (1.5 * median).min(33.3)
    } else {
        33.3
    };

    frames
        .iter()
        .filter(|f| f.frametime_ms.is_finite() && f.frametime_ms > threshold)
        .cloned()
        .collect()
}

pub(crate) fn build_frame_pacing_summary(
    frame_events: &[FrameEvent],
    clusters: &[SpikeCluster],
    foreground_events: &[ForegroundEvent],
    max_foreground_stale_ms: u64,
) -> FramePacingSummary {
    let mut frametimes = frame_events
        .iter()
        .filter_map(|frame| frame.frametime_ms.is_finite().then_some(frame.frametime_ms))
        .collect::<Vec<_>>();

    if frametimes.is_empty() {
        return FramePacingSummary {
            frame_count: frame_events.len(),
            notes: vec![
                "No frame events loaded; pass --mangohud-log to enable frame-pacing views."
                    .to_owned(),
            ],
            ..Default::default()
        };
    }

    let median = median_f64(&mut frametimes);
    let p95 = percentile_f64(&mut frametimes.clone(), 0.95);
    let p99 = percentile_f64(&mut frametimes.clone(), 0.99);
    let max = frametimes.iter().copied().fold(0.0_f64, f64::max);

    let compositor_cluster_count = clusters
        .iter()
        .filter(|cluster| {
            cluster
                .anchor_class
                .is_some_and(is_compositor_frame_pacing_class)
        })
        .count();
    let game_cluster_count = clusters
        .iter()
        .filter(|cluster| cluster.anchor_class.is_some_and(is_game_frame_pacing_class))
        .count();

    let mut outliers = Vec::new();
    let mut outlier_count = 0;
    let mut sorted_frames = frame_events
        .iter()
        .filter(|frame| frame.frametime_ms.is_finite())
        .collect::<Vec<_>>();
    sorted_frames.sort_by_key(|frame| frame.elapsed_ms);

    for frame in sorted_frames {
        let over_median_ratio = (median > 0.0).then_some(frame.frametime_ms / median);
        let is_outlier =
            over_median_ratio.is_some_and(|ratio| ratio >= 2.0) || frame.frametime_ms >= 33.3;

        if !is_outlier {
            continue;
        }

        outlier_count += 1;

        let nearest_cluster = nearest_cluster_for_elapsed(frame.elapsed_ms, clusters);
        let foreground =
            foreground_for_elapsed_ms(frame.elapsed_ms, foreground_events, max_foreground_stale_ms);

        outliers.push(FrameOutlierView {
            elapsed_ms: frame.elapsed_ms,
            frametime_ms: frame.frametime_ms,
            over_median_ratio,
            nearest_cluster_delta_ms: nearest_cluster
                .as_ref()
                .map(|(_, elapsed_ms)| signed_ms_delta(*elapsed_ms, frame.elapsed_ms)),
            nearest_cluster_cause: nearest_cluster
                .as_ref()
                .and_then(|(cluster, _)| cluster.diagnosis.as_ref())
                .and_then(diagnosis_cause_label),
            nearest_cluster_anchor_class: nearest_cluster
                .as_ref()
                .and_then(|(cluster, _)| cluster.anchor_class),
            nearest_cluster_anchor_comm: nearest_cluster
                .as_ref()
                .and_then(|(cluster, _)| cluster.anchor_comm.clone()),
            foreground_pid: foreground
                .and_then(|event| event.decision.target.as_ref().and_then(|t| t.pid)),
            foreground_app_id: foreground.and_then(|event| {
                event
                    .decision
                    .target
                    .as_ref()
                    .and_then(|t| t.app_id.clone())
                    .clone()
            }),
            foreground_class: foreground.and_then(|event| {
                event
                    .decision
                    .target
                    .as_ref()
                    .and_then(|t| t.class.clone())
                    .clone()
            }),
        });
    }

    let mut notes = Vec::new();
    if outlier_count == 0 {
        notes.push("No frame-pacing outliers crossed the display threshold.".to_owned());
    }
    if compositor_cluster_count > 0 {
        notes.push(format!(
            "{compositor_cluster_count} scheduler cluster(s) were anchored on compositor/gamescope tasks."
        ));
    }
    if game_cluster_count > 0 {
        notes.push(format!(
            "{game_cluster_count} scheduler cluster(s) were anchored on game tasks."
        ));
    }

    FramePacingSummary {
        frame_count: frame_events.len(),
        median_frametime_ms: Some(median),
        p95_frametime_ms: Some(p95),
        p99_frametime_ms: Some(p99),
        max_frametime_ms: Some(max),
        outlier_count,
        outliers,
        compositor_cluster_count,
        game_cluster_count,
        notes,
    }
}

pub(crate) fn nearest_cluster_for_elapsed(
    elapsed_ms: u64,
    clusters: &[SpikeCluster],
) -> Option<(&SpikeCluster, u64)> {
    clusters
        .iter()
        .filter_map(|cluster| {
            cluster_elapsed(cluster).map(|cluster_elapsed_ms| {
                (
                    cluster,
                    cluster_elapsed_ms,
                    cluster_elapsed_ms.abs_diff(elapsed_ms),
                )
            })
        })
        .min_by_key(|(_, _, delta)| *delta)
        .map(|(cluster, cluster_elapsed_ms, _)| (cluster, cluster_elapsed_ms))
}

pub(crate) fn signed_ms_delta(cluster_elapsed_ms: u64, frame_elapsed_ms: u64) -> i64 {
    if cluster_elapsed_ms >= frame_elapsed_ms {
        (cluster_elapsed_ms - frame_elapsed_ms) as i64
    } else {
        -((frame_elapsed_ms - cluster_elapsed_ms) as i64)
    }
}

pub(crate) fn diagnosis_cause_label(diagnosis: &Diagnosis) -> Option<String> {
    diagnosis
        .primary
        .as_ref()
        .map(|primary| format!("{:?}", primary.cause))
        .or_else(|| Some(format!("{:?}", diagnosis.cause)))
}

pub(crate) fn is_compositor_frame_pacing_class(class: TaskClass) -> bool {
    matches!(class, TaskClass::Compositor | TaskClass::GameScope)
}

pub(crate) fn is_game_frame_pacing_class(class: TaskClass) -> bool {
    matches!(
        class,
        TaskClass::Game
            | TaskClass::GameRenderThread
            | TaskClass::GameWorkerThread
            | TaskClass::GameHelper
            | TaskClass::WineServer
    )
}

pub(crate) fn perform_frame_diagnosis(
    session: &SessionFile,
    frame_spikes: &[FrameEvent],
    all_spike_points: &[SpikePoint],
    artifacts: &session_io::RunArtifacts,
    cluster_window_ns: u64,
) -> Vec<FrameDiagnosis> {
    let mut diagnoses = Vec::new();
    for frame in frame_spikes {
        let frame_monotonic_ns = if let Some(start_ns) = session.core.monotonic_start_ns {
            start_ns + (frame.elapsed_ms * 1_000_000)
        } else {
            0
        };

        let nearby_points: Vec<_> = all_spike_points
            .iter()
            .filter(|p| p.switch_ns.abs_diff(frame_monotonic_ns) <= cluster_window_ns)
            .cloned()
            .collect();

        let distinct_tasks = nearby_points
            .iter()
            .map(|p| p.task)
            .collect::<BTreeSet<_>>()
            .len();
        let cluster = cluster_from_points(nearby_points, distinct_tasks);

        // Let `diagnose_cluster` handle artifact filtering.

        let diagnosis = diagnose_cluster(&cluster, artifacts, cluster_window_ns);

        diagnoses.push(FrameDiagnosis {
            frame_elapsed_ms: frame.elapsed_ms,
            frametime_ms: frame.frametime_ms,
            diagnosis,
        });
    }

    diagnoses
}

pub(crate) fn compute_correlation_windows(
    session: &SessionFile,
    clusters: &[SpikeCluster],
    frame_spikes: &[FrameEvent],
    cluster_window_ns: u64,
) -> session_io::CorrelationWindows {
    let mut windows = session_io::CorrelationWindows::default();
    let padding_ms = (cluster_window_ns / 1_000_000).max(1);

    for cluster in clusters {
        let min_ns = cluster.min_switch_ns.saturating_sub(cluster_window_ns);
        let max_ns = cluster.max_switch_ns.saturating_add(cluster_window_ns);
        windows.windows_ns.push((min_ns, max_ns));

        if let Some((min_e, max_e)) = cluster_elapsed_range(cluster) {
            // Padding for SCX (2000ms), CPU freq (50ms), GPU (50ms), intervals (1000ms)
            windows.windows_ms.push((
                min_e.saturating_sub(2000).max(padding_ms),
                max_e.saturating_add(2000).max(padding_ms),
            ));
        }
    }

    for frame in frame_spikes {
        if let Some(start_ns) = session.core.monotonic_start_ns {
            let frame_ns = start_ns + (frame.elapsed_ms * 1_000_000);
            windows.windows_ns.push((
                frame_ns.saturating_sub(cluster_window_ns),
                frame_ns.saturating_add(cluster_window_ns),
            ));
        }
        windows.windows_ms.push((
            frame.elapsed_ms.saturating_sub(2000).max(padding_ms),
            frame.elapsed_ms.saturating_add(2000).max(padding_ms),
        ));
    }

    windows
}
