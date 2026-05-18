use super::*;
use crate::{
    artifacts::{ArtifactKind, artifact_file_name},
    sched_state::classify_switch_prev_state,
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

pub(crate) fn foreground_report_summary(
    session: &SessionFile,
    foreground_events: &[ForegroundEvent],
) -> ForegroundReportSummary {
    let final_event = foreground_events.last();

    let enabled = session.config.foreground_window || session.core.foreground_event_count > 0;
    let source = final_event
        .map(|event| format!("{:?}", event.source).to_ascii_lowercase())
        .or_else(|| session.core.foreground_source.clone())
        .or_else(|| {
            (!session.config.foreground_source.is_empty())
                .then(|| session.config.foreground_source.clone())
        });

    let final_pid = final_event
        .and_then(|event| event.pid)
        .or(session.core.final_foreground_pid);
    let final_app_id = final_event
        .and_then(|event| event.app_id.clone())
        .or_else(|| session.core.final_foreground_app_id.clone());
    let final_class = final_event
        .and_then(|event| event.class.clone())
        .or_else(|| session.core.final_foreground_class.clone());
    let final_title = final_event.and_then(|event| event.title.clone());
    let final_workspace = final_event.and_then(|event| event.workspace.clone());
    let confidence = final_event.map(|event| event.confidence);
    let provider_status =
        final_event.map(|event| format!("{:?}", event.status).to_ascii_lowercase());
    let reasons = final_event
        .map(|event| vec![event.reason.clone()])
        .unwrap_or_default();

    ForegroundReportSummary {
        enabled,
        source,
        final_pid,
        final_app_id,
        final_class,
        final_title,
        final_workspace,
        event_count: session
            .core
            .foreground_event_count
            .max(foreground_events.len() as u64),
        confidence,
        provider_status,
        reasons,
    }
}

pub(crate) fn focus_report_summary(
    session: &SessionFile,
    focus_events: &[FocusEvent],
) -> FocusReportSummary {
    let final_event = focus_events
        .iter()
        .rev()
        .find(|event| event.action == "changed" || event.kind.is_some());

    let mode = session
        .core
        .focus_mode
        .clone()
        .or_else(|| session.config.auto_focus.then(|| "auto-focus".to_owned()));

    let final_focus = final_event
        .and_then(|event| event.kind.clone())
        .or_else(|| session.core.final_focus_kind.clone());

    let situation =
        final_event.and_then(|event| event.situation.map(|situation| format!("{situation:?}")));

    let confidence = final_event.map(|event| event.confidence);
    let score = final_event.map(|event| event.score);
    let roots = final_event
        .map(|event| event.root_pids.clone())
        .unwrap_or_default();
    let member_pids = final_event
        .map(|event| event.member_pids.clone())
        .unwrap_or_default();
    let reasons = final_event
        .map(|event| event.reasons.clone())
        .unwrap_or_default();

    let display_name = final_focus.clone();

    FocusReportSummary {
        mode,
        final_focus,
        display_name,
        situation,
        confidence,
        score,
        roots,
        member_pids,
        focus_switches: session.core.focus_switch_count,
        reasons,
    }
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

pub(crate) fn ms_to_ns_i64(value: f64) -> i64 {
    (value * 1_000_000.0).ceil() as i64
}

pub(crate) fn top_task_rows_by_max_latency(
    session: &SessionFile,
    top: usize,
    filter_class: Option<TaskClass>,
) -> Vec<TaskHtmlRow> {
    let mut tasks = filtered_latency_tasks(session, filter_class);
    tasks.sort_by_key(|task| std::cmp::Reverse(task.latency.max_ns));
    tasks.into_iter().take(top).map(task_html_row).collect()
}

pub(crate) fn top_task_rows_by_p99_latency(
    session: &SessionFile,
    top: usize,
    filter_class: Option<TaskClass>,
) -> Vec<TaskHtmlRow> {
    let mut tasks = filtered_latency_tasks(session, filter_class);
    tasks.sort_by_key(|task| {
        (
            std::cmp::Reverse(task.latency.p99_ns),
            std::cmp::Reverse(task.latency.max_ns),
        )
    });
    tasks.into_iter().take(top).map(task_html_row).collect()
}

pub(crate) fn filtered_latency_tasks(
    session: &SessionFile,
    filter_class: Option<TaskClass>,
) -> Vec<&SessionTask> {
    session
        .tasks
        .iter()
        .filter(|task| task.latency.samples > 0)
        .filter(|task| filter_class.is_none_or(|class| task.class == class))
        .collect()
}

pub(crate) fn ns_to_ms(ns: u64) -> f64 {
    ns as f64 / 1_000_000.0
}

pub fn build_spike_density(spikes: &[SpikeEvent], bucket_ms: u64) -> Vec<SpikeDensityBucket> {
    if spikes.is_empty() {
        return Vec::new();
    }

    let bucket_ms = bucket_ms.max(1);

    #[derive(Default)]
    struct BucketAccum {
        start_ms: u64,
        end_ms: u64,
        count: u64,
        max_latency_ms: f64,
        latencies_ms: Vec<f64>,
    }

    let mut buckets: BTreeMap<u64, BucketAccum> = BTreeMap::new();

    for spike in spikes {
        let elapsed_ms = spike.elapsed_ms.unwrap_or(0);
        let latency_ms = spike.latency_ns as f64 / 1_000_000.0;

        let bucket_idx = elapsed_ms / bucket_ms;
        let start_ms = bucket_idx * bucket_ms;
        let end_ms = start_ms + bucket_ms;

        let bucket = buckets.entry(bucket_idx).or_insert_with(|| BucketAccum {
            start_ms,
            end_ms,
            count: 0,
            max_latency_ms: 0.0,
            latencies_ms: Vec::new(),
        });

        bucket.count += 1;
        if latency_ms.is_finite() {
            bucket.max_latency_ms = bucket.max_latency_ms.max(latency_ms);
            bucket.latencies_ms.push(latency_ms);
        }
    }

    buckets
        .into_values()
        .map(|mut bucket| {
            let p99_latency_ms = percentile_f64(&mut bucket.latencies_ms, 0.99);
            SpikeDensityBucket {
                start_ms: bucket.start_ms,
                end_ms: bucket.end_ms,
                count: bucket.count,
                max_latency_ms: bucket.max_latency_ms,
                p99_latency_ms,
            }
        })
        .collect()
}

pub(crate) fn percentile_f64(values: &mut [f64], percentile: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }

    values.sort_by(|a, b| a.total_cmp(b));

    let len = values.len();
    let rank = ((len as f64 - 1.0) * percentile).round() as usize;
    values[rank.min(len - 1)]
}

pub(crate) fn text_report_correlation_sections(
    clusters: &[SpikeCluster],
    artifacts: &session_io::RunArtifacts,
    block_io_correlation_basis: &str,
    cluster_window_ns: u64,
    top: usize,
) -> TextReportCorrelationSections {
    let mut sections = TextReportCorrelationSections::new();

    let min_overall = clusters
        .iter()
        .map(|cluster| cluster.min_switch_ns.saturating_sub(cluster_window_ns))
        .min()
        .unwrap_or(0);
    let max_overall = clusters
        .iter()
        .map(|cluster| cluster.max_switch_ns.saturating_add(cluster_window_ns))
        .max()
        .unwrap_or(0);

    if let Some(section) = build_text_correlation_section(
        clusters,
        top,
        "irq overlap",
        &artifacts.irq_events,
        |event| event.exit_ns >= min_overall && event.enter_ns <= max_overall,
        |cluster, event| {
            let min_ns = cluster.min_switch_ns.saturating_sub(cluster_window_ns);
            let max_ns = cluster.max_switch_ns.saturating_add(cluster_window_ns);
            event.exit_ns >= min_ns && event.enter_ns <= max_ns
        },
        |rank, cluster, matches| {
            let max_duration = matches
                .iter()
                .map(|event| event.duration_ns)
                .max()
                .unwrap_or(0);
            let irq_list = matches
                .iter()
                .map(|event| event.irq)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .map(|irq| irq.to_string())
                .collect::<Vec<_>>()
                .join(",");
            let min_ns = cluster.min_switch_ns.saturating_sub(cluster_window_ns);
            let max_ns = cluster.max_switch_ns.saturating_add(cluster_window_ns);
            vec![format!(
                "cluster=#{} matches={} irqs={} max_duration={} window_ns={}..{}",
                rank + 1,
                matches.len(),
                irq_list,
                format_latency(max_duration),
                min_ns,
                max_ns
            )]
        },
    ) {
        sections.push_section(section);
    }

    let min_overall_opt = clusters.iter().filter_map(cluster_elapsed).min();
    let max_overall_opt = clusters.iter().filter_map(cluster_elapsed).max();
    if let (Some(min_overall), Some(max_overall)) = (min_overall_opt, max_overall_opt) {
        let lower = min_overall.saturating_sub(50);
        let upper = max_overall.saturating_add(50);
        if let Some(section) = build_text_correlation_section(
            clusters,
            top,
            "gpu near clusters",
            &artifacts.gpu_samples,
            |sample| sample.elapsed_ms >= lower && sample.elapsed_ms <= upper,
            |cluster, sample| {
                cluster_elapsed(cluster)
                    .is_some_and(|elapsed| sample.elapsed_ms.abs_diff(elapsed) <= 50)
            },
            |rank, cluster, matches| {
                let elapsed = cluster_elapsed(cluster).unwrap();
                let sample = matches
                    .iter()
                    .min_by_key(|sample| sample.elapsed_ms.abs_diff(elapsed))
                    .unwrap();
                vec![format!(
                    "cluster=#{} sample_elapsed={} gpu_busy={} gpu_clock_mhz={} mem_clock_mhz={} temp_mC={} power_uW={}",
                    rank + 1,
                    format_elapsed(Some(sample.elapsed_ms)),
                    format_option(sample.gpu_busy_percent),
                    format_option(sample.gpu_clock_mhz),
                    format_option(sample.mem_clock_mhz),
                    format_option(sample.temp_millidegrees),
                    format_option(sample.power_microwatts),
                )]
            },
        ) {
            sections.push_section(section);
        }
    }

    let padding_ms = (cluster_window_ns / 1_000_000).max(1);
    let min_overall_opt = clusters
        .iter()
        .filter_map(|cluster| cluster_elapsed_range(cluster).map(|(min, _)| min))
        .min();
    let max_overall_opt = clusters
        .iter()
        .filter_map(|cluster| cluster_elapsed_range(cluster).map(|(_, max)| max))
        .max();
    if let (Some(min_overall), Some(max_overall)) = (min_overall_opt, max_overall_opt) {
        let lower = min_overall.saturating_sub(padding_ms);
        let upper = max_overall.saturating_add(padding_ms);
        if let Some(section) = build_text_correlation_section(
            clusters,
            top,
            "frame overlap",
            &artifacts.frame_events,
            |frame| frame.elapsed_ms >= lower && frame.elapsed_ms <= upper,
            |cluster, frame| {
                cluster_elapsed_range(cluster).is_some_and(|(min_elapsed, max_elapsed)| {
                    let min_elapsed = min_elapsed.saturating_sub(padding_ms);
                    let max_elapsed = max_elapsed.saturating_add(padding_ms);
                    frame.elapsed_ms >= min_elapsed && frame.elapsed_ms <= max_elapsed
                })
            },
            |rank, cluster, matches| {
                let (min_elapsed, max_elapsed) = cluster_elapsed_range(cluster).unwrap();
                let min_elapsed = min_elapsed.saturating_sub(padding_ms);
                let max_elapsed = max_elapsed.saturating_add(padding_ms);
                let max_frame = matches
                    .iter()
                    .map(|frame| frame.frametime_ms)
                    .fold(0.0_f64, f64::max);
                vec![format!(
                    "cluster=#{} frames={} max_frametime_ms={:.3} elapsed={}..{}",
                    rank + 1,
                    matches.len(),
                    max_frame,
                    min_elapsed,
                    max_elapsed
                )]
            },
        ) {
            sections.push_section(section);
        }
    }

    let min_overall = clusters
        .iter()
        .map(|cluster| cluster.min_switch_ns.saturating_sub(cluster_window_ns))
        .min()
        .unwrap_or(0);
    let max_overall = clusters
        .iter()
        .map(|cluster| cluster.max_switch_ns.saturating_add(cluster_window_ns))
        .max()
        .unwrap_or(0);

    if let Some(section) = build_text_correlation_section(
        clusters,
        top,
        "migration overlap",
        &artifacts.migration_events,
        |event| event.timestamp_ns >= min_overall && event.timestamp_ns <= max_overall,
        |cluster, event| {
            let min_ns = cluster.min_switch_ns.saturating_sub(cluster_window_ns);
            let max_ns = cluster.max_switch_ns.saturating_add(cluster_window_ns);
            event.timestamp_ns >= min_ns && event.timestamp_ns <= max_ns
        },
        |rank, cluster, matches| {
            let tids = matches
                .iter()
                .map(|event| event.tid)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .map(|tid| tid.to_string())
                .collect::<Vec<_>>()
                .join(",");
            let min_ns = cluster.min_switch_ns.saturating_sub(cluster_window_ns);
            let max_ns = cluster.max_switch_ns.saturating_add(cluster_window_ns);
            vec![format!(
                "cluster=#{} matches={} tids={} window_ns={}..{}",
                rank + 1,
                matches.len(),
                tids,
                min_ns,
                max_ns
            )]
        },
    ) {
        sections.push_section(section);
    }

    let min_overall_opt = clusters.iter().filter_map(cluster_elapsed).min();
    let max_overall_opt = clusters.iter().filter_map(cluster_elapsed).max();
    if let (Some(min_overall), Some(max_overall)) = (min_overall_opt, max_overall_opt) {
        let lower = min_overall.saturating_sub(50);
        let upper = max_overall.saturating_add(50);
        if let Some(section) = build_text_correlation_section(
            clusters,
            top,
            "cpu freq near clusters",
            &artifacts.cpu_freq_events,
            |sample| sample.elapsed_ms >= lower && sample.elapsed_ms <= upper,
            |cluster, sample| {
                cluster_elapsed(cluster)
                    .is_some_and(|elapsed| sample.elapsed_ms.abs_diff(elapsed) <= 50)
            },
            |rank, _, matches| {
                let max_freq = matches
                    .iter()
                    .map(|sample| sample.freq_khz)
                    .max()
                    .unwrap_or(0);
                vec![format!(
                    "cluster=#{} cpu_freq_samples={} max_freq_khz={}",
                    rank + 1,
                    matches.len(),
                    max_freq
                )]
            },
        ) {
            sections.push_section(section);
        }
    }

    let min_overall = clusters
        .iter()
        .map(|cluster| cluster.min_switch_ns.saturating_sub(cluster_window_ns))
        .min()
        .unwrap_or(0);
    let max_overall = clusters
        .iter()
        .map(|cluster| cluster.max_switch_ns.saturating_add(cluster_window_ns))
        .max()
        .unwrap_or(0);

    let io_title = if block_io_correlation_basis == "dev+sector" {
        "block i/o overlap (advisory, approximate; correlated by dev+sector)"
    } else {
        "block i/o overlap (correlated by request-pointer)"
    };

    if let Some(section) = build_text_correlation_section(
        clusters,
        top,
        io_title,
        &artifacts.block_io_events,
        |event| {
            event.timestamp_ns >= min_overall
                && event.timestamp_ns.saturating_sub(event.duration_ns) <= max_overall
        },
        |cluster, event| {
            let min_ns = cluster.min_switch_ns.saturating_sub(cluster_window_ns);
            let max_ns = cluster.max_switch_ns.saturating_add(cluster_window_ns);
            event.timestamp_ns >= min_ns
                && event.timestamp_ns.saturating_sub(event.duration_ns) <= max_ns
        },
        |rank, cluster, matches| {
            let max_duration = matches
                .iter()
                .map(|event| event.duration_ns)
                .max()
                .unwrap_or(0);
            let tids = matches
                .iter()
                .map(|event| event.tid)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .map(|tid| tid.to_string())
                .collect::<Vec<_>>()
                .join(",");
            let min_ns = cluster.min_switch_ns.saturating_sub(cluster_window_ns);
            let max_ns = cluster.max_switch_ns.saturating_add(cluster_window_ns);
            vec![format!(
                "cluster=#{} matches={} tids={}{} max_duration={} window_ns={}..{}",
                rank + 1,
                matches.len(),
                tids,
                if block_io_correlation_basis == "dev+sector" {
                    " (approximate)"
                } else {
                    ""
                },
                format_latency(max_duration),
                min_ns,
                max_ns
            )]
        },
    ) {
        sections.push_section(section);
    }

    let min_overall_opt = clusters.iter().filter_map(cluster_elapsed).min();
    let max_overall_opt = clusters.iter().filter_map(cluster_elapsed).max();
    if let (Some(min_overall), Some(max_overall)) = (min_overall_opt, max_overall_opt) {
        let lower = min_overall.saturating_sub(2000);
        let upper = max_overall.saturating_add(2000);
        if let Some(section) = build_text_correlation_section(
            clusters,
            top,
            "scx transitions near clusters",
            &artifacts.scx_events,
            |event| event.elapsed_ms >= lower && event.elapsed_ms <= upper,
            |cluster, event| {
                cluster_elapsed(cluster)
                    .is_some_and(|elapsed| event.elapsed_ms.abs_diff(elapsed) <= 2000)
            },
            |rank, cluster, matches| {
                let elapsed = cluster_elapsed(cluster).unwrap();
                matches
                    .iter()
                    .map(|event| {
                        format!(
                            "cluster=#{} SCX transition near spike: ops={} state={} at elapsed={}ms (cluster_elapsed={}ms)",
                            rank + 1,
                            event.ops.as_deref().unwrap_or("-"),
                            event.state.as_deref().unwrap_or("-"),
                            event.elapsed_ms,
                            elapsed
                        )
                    })
                    .collect()
            },
        ) {
            sections.push_section(section);
        }
    }

    sections
}

fn build_text_correlation_section<T, LP, MP, R>(
    clusters: &[SpikeCluster],
    top: usize,
    title: &str,
    in_memory: &[T],
    mut load_predicate: LP,
    mut match_predicate: MP,
    mut build_lines: R,
) -> Option<TextReportCorrelationSection>
where
    LP: FnMut(&T) -> bool,
    MP: FnMut(&SpikeCluster, &T) -> bool,
    R: FnMut(usize, &SpikeCluster, &[&T]) -> Vec<String>,
{
    let pool = in_memory
        .iter()
        .filter(|item| load_predicate(*item))
        .collect::<Vec<_>>();

    if pool.is_empty() {
        return None;
    }

    let mut section = TextReportCorrelationSection::new(title);

    for (rank, cluster) in clusters.iter().take(top).enumerate() {
        let matches = pool
            .iter()
            .copied()
            .filter(|item| match_predicate(cluster, *item))
            .collect::<Vec<_>>();

        if !matches.is_empty() {
            for line in build_lines(rank, cluster, &matches) {
                section.push_line(line);
            }
        }
    }

    Some(section)
}

pub(crate) fn runtime_slice_analysis_summary(
    session: &SessionFile,
    artifacts: &session_io::RunArtifacts,
) -> RuntimeSliceAnalysisSummary {
    let mut summary = RuntimeSliceAnalysisSummary {
        sample_count: artifacts.runtime_slices.len(),
        available: !artifacts.runtime_slices.is_empty(),
        ..Default::default()
    };

    if !session.config.runtime_slices && session.core.runtime_slice_count == 0 {
        summary.missing_reason = Some("runtime-slice collection was not enabled".to_owned());
    } else if session.config.runtime_slices && session.core.runtime_slice_count == 0 {
        summary.missing_reason = Some("no runtime slice samples were recorded".to_owned());
    } else if artifacts
        .validation
        .missing_optional_files
        .iter()
        .any(|file| file == artifact_file_name(ArtifactKind::RuntimeSlices))
    {
        summary.missing_reason = Some("runtime_slices.json is missing".to_owned());
    } else if session.core.runtime_slice_count > 0 && artifacts.runtime_slices.is_empty() {
        summary.missing_reason =
            Some("runtime slices exist, but none overlapped report correlation windows".to_owned());
    }

    if session.core.runtime_slice_read_errors > 0 {
        summary.data_quality_notes.push(format!(
            "runtime slice sampler had {} read errors",
            session.core.runtime_slice_read_errors
        ));
    }
    if session.core.runtime_slice_skipped_tasks > 0 {
        summary.data_quality_notes.push(format!(
            "runtime slice sampler skipped {} tasks due to runtime_slices_max_tasks",
            session.core.runtime_slice_skipped_tasks
        ));
    }
    if artifacts.runtime_slices.iter().any(|record| {
        matches!(
            record.source,
            crate::metrics::RuntimeSliceSource::ProcStatFallback
        )
    }) {
        summary.data_quality_notes.push(
            "some runtime slices used /proc stat fallback without runqueue wait data".to_owned(),
        );
    }
    if summary.available {
        summary.notes.push(
            "runtime slice context is supporting evidence, not a primary diagnosis by itself"
                .to_owned(),
        );
    }

    for record in artifacts
        .runtime_slices
        .iter()
        .filter(|record| record.valid)
    {
        *summary
            .source_counts
            .entry(record.source.as_str().to_owned())
            .or_insert(0) += 1;
    }

    summary.high_runtime_threads = top_runtime_threads(&artifacts.runtime_slices, true);
    summary.high_wait_threads = top_runtime_threads(&artifacts.runtime_slices, false);
    summary
}

pub(crate) fn top_runtime_threads(
    records: &[crate::metrics::RuntimeSliceRecord],
    by_runtime: bool,
) -> Vec<RuntimeThreadSummary> {
    let mut best_by_task: BTreeMap<u32, RuntimeThreadSummary> = BTreeMap::new();

    for record in records.iter().filter(|record| record.valid) {
        let score = if by_runtime {
            record.runtime_ratio.unwrap_or(0.0)
        } else {
            record.wait_ratio.unwrap_or(0.0)
        };

        if score <= 0.0 {
            continue;
        }

        let candidate = RuntimeThreadSummary {
            task: record.task,
            process_pid: record.process_pid,
            class: record.class,
            comm: record.comm.clone(),
            process_comm: record.process_comm.to_string(),
            max_runtime_ratio: record.runtime_ratio.unwrap_or(0.0),
            max_wait_ratio: record.wait_ratio,
            max_runtime_delta_ns: record.runtime_delta_ns,
            max_runqueue_wait_delta_ns: record.runqueue_wait_delta_ns,
            elapsed_ms: record.elapsed_ms,
        };

        match best_by_task.get(&record.task) {
            Some(existing) => {
                let existing_score = if by_runtime {
                    existing.max_runtime_ratio
                } else {
                    existing.max_wait_ratio.unwrap_or(0.0)
                };
                if score > existing_score {
                    best_by_task.insert(record.task, candidate);
                }
            }
            None => {
                best_by_task.insert(record.task, candidate);
            }
        }
    }

    let mut rows = best_by_task.into_values().collect::<Vec<_>>();
    rows.sort_by(|a, b| {
        let left = if by_runtime {
            a.max_runtime_ratio
        } else {
            a.max_wait_ratio.unwrap_or(0.0)
        };
        let right = if by_runtime {
            b.max_runtime_ratio
        } else {
            b.max_wait_ratio.unwrap_or(0.0)
        };
        right
            .partial_cmp(&left)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.task.cmp(&b.task))
    });
    rows.truncate(10);
    rows
}

pub(crate) fn format_optional_ratio(value: Option<f64>) -> String {
    value
        .map(|ratio| format!("{:.1}%", ratio * 100.0))
        .unwrap_or_else(|| "-".to_owned())
}

pub(crate) fn format_pressure_option(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.2}"))
        .unwrap_or_else(|| "-".to_owned())
}

pub(crate) fn block_io_correlation_basis(session: &SessionFile) -> &str {
    if session.core.block_io_correlation_basis.is_empty() {
        "dev+sector"
    } else {
        &session.core.block_io_correlation_basis
    }
}

pub(crate) fn block_io_correlation_confidence(session: &SessionFile) -> &str {
    if session.core.block_io_correlation_confidence.is_empty() {
        crate::ebpf_loader::BlockIoCorrelationBasis::from_str(block_io_correlation_basis(session))
            .confidence()
    } else {
        &session.core.block_io_correlation_confidence
    }
}

pub(crate) fn block_io_correlation_warning(session: &SessionFile) -> Option<String> {
    match block_io_correlation_basis(session) {
        "dev+sector" => crate::ebpf_loader::BlockIoCorrelationBasis::DevSector
            .warning()
            .map(str::to_owned),
        _ => None,
    }
}

pub(crate) fn data_quality_summary(
    session: &SessionFile,
    validation: &crate::session_io::RunValidationReport,
) -> DataQualitySummary {
    let mut reasons = Vec::new();
    let mut level = DataQualityLevel::High;

    if !validation.errors.is_empty() {
        level = DataQualityLevel::Low;
        reasons.push("run directory has validation errors".to_owned());
    }

    if session.core.schema_version != SESSION_SCHEMA_VERSION {
        if session.core.schema_version > SESSION_SCHEMA_VERSION {
            level = DataQualityLevel::Low;
            reasons.push("session schema is newer than this stutter binary".to_owned());
        } else {
            level = downgrade_quality(level, DataQualityLevel::Medium);
            reasons.push("session schema is older than this stutter binary".to_owned());
        }
    }

    if session.core.event_stream_write_errors > 0 {
        level = DataQualityLevel::Low;
        reasons.push("recording stream had write errors".to_owned());
    }

    let missing_non_focus_optional = validation
        .missing_optional_files
        .iter()
        .filter(|f| {
            *f != artifact_file_name(ArtifactKind::FocusEvents)
                && *f != artifact_file_name(ArtifactKind::ForegroundEvents)
        })
        .collect::<Vec<_>>();

    if !missing_non_focus_optional.is_empty() {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push("optional correlation artifacts are missing".to_owned());
    }

    if session.core.spike_events_truncated {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push("spike event stream was truncated".to_owned());
    }

    if session.core.spike_events_dropped_count > 0 {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push("spike events were dropped".to_owned());
    }

    if session.core.interval_record_count == 0 {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push("no interval records are available".to_owned());
    }

    if session.core.active_target_pids_count == 0 {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push("no active target tasks were present at end of run".to_owned());
    }

    let drop_counters_nonzero = session.core.drop_counters.total() > 0;

    if drop_counters_nonzero {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push("eBPF drop counters are non-zero".to_owned());
    }

    let mut percentile_scope_counts = BTreeMap::new();
    for task in &session.tasks {
        *percentile_scope_counts
            .entry(task.latency.percentile_scope.clone())
            .or_insert(0) += 1;
    }

    if percentile_scope_counts.contains_key("histogram") {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push("some percentile values are histogram-estimated".to_owned());
    }

    if percentile_scope_counts.contains_key("capped_prefix") {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push("some percentile values are based on capped prefix samples".to_owned());
    }

    let block_io_correlation_basis = block_io_correlation_basis(session).to_owned();
    let block_io_correlation_confidence = block_io_correlation_confidence(session).to_owned();
    let block_io_correlation_warning = block_io_correlation_warning(session);
    if session.core.block_io_event_count > 0 && block_io_correlation_basis == "dev+sector" {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        if let Some(warning) = &block_io_correlation_warning {
            reasons.push(warning.clone());
        } else {
            reasons.push("block I/O correlation is approximate dev+sector matching".to_owned());
        }
    }

    let frame_timestamp_alignment = if session.core.frame_event_count == 0 {
        "none".to_owned()
    } else if session.core.mangohud_first_frame_monotonic_ns.is_some() {
        "monotonic_observed".to_owned()
    } else {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push("MangoHud frame timestamp alignment is approximate".to_owned());
        "approximate_first_row".to_owned()
    };

    let cpu_perf_requested = session.config.cpu_perf;
    let task_cpu_perf_count = session
        .tasks
        .iter()
        .filter(|task| task.cpu_perf.is_some())
        .count();
    if cpu_perf_requested && task_cpu_perf_count == 0 {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push("CPU perf was requested but no counters were recorded".to_owned());
    }
    if session.core.cpu_perf_open_errors > 0 || session.core.cpu_perf_read_errors > 0 {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push("CPU perf counters had open/read errors".to_owned());
    }
    if session.core.cpu_perf_skipped_tasks > 0 {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push(format!(
            "CPU perf skipped {} active tasks due to cpu_perf_max_tasks limit",
            session.core.cpu_perf_skipped_tasks
        ));
    }
    if session
        .tasks
        .iter()
        .filter_map(|task| task.cpu_perf.as_ref())
        .any(|perf| perf.multiplexed)
    {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push("CPU perf counters were multiplexed; values are scaled estimates".to_owned());
    }

    if reasons.is_empty() {
        reasons.push("no data-quality problems detected".to_owned());
    }

    DataQualitySummary {
        level,
        reasons,
        missing_optional_files: validation.missing_optional_files.clone(),
        validation_errors: validation.errors.clone(),
        validation_warnings: validation.warnings.clone(),
        schema_version: session.core.schema_version,
        expected_schema_version: SESSION_SCHEMA_VERSION,
        event_stream_write_errors: session.core.event_stream_write_errors,
        spike_events_truncated: session.core.spike_events_truncated,
        spike_events_retained_count: session.core.spike_events_retained_count,
        spike_events_dropped_count: session.core.spike_events_dropped_count,
        interval_record_count: session.core.interval_record_count,
        active_target_pids_count: session.core.active_target_pids_count,
        drop_counters_nonzero,
        percentile_scope_counts,
        block_io_correlation_basis,
        block_io_correlation_confidence,
        block_io_correlation_warning,
        frame_timestamp_alignment,
        cpu_perf_requested,
        cpu_perf_open_errors: session.core.cpu_perf_open_errors,
        cpu_perf_read_errors: session.core.cpu_perf_read_errors,
        cpu_perf_skipped_tasks: session.core.cpu_perf_skipped_tasks,
    }
}

pub(crate) fn build_pressure_timeline(
    intervals: &[IntervalRecord],
    clusters: &[SpikeCluster],
    cluster_window_ms: u64,
) -> PressureTimelineSummary {
    if intervals.is_empty() {
        return PressureTimelineSummary {
            sample_count: 0,
            max_cpu_some: 0.0,
            max_mem_some: None,
            max_mem_full: None,
            max_io_some: None,
            max_io_full: None,
            windows: Vec::new(),
            peak_windows: Vec::new(),
            pressure_notes: vec![
                "No interval records loaded; pressure timeline unavailable".to_owned(),
            ],
            coverage: PressureTimelineCoverage::default(),
        };
    }

    let mut sorted_intervals = intervals.iter().collect::<Vec<_>>();
    sorted_intervals.sort_by_key(|record| record.elapsed_ms);

    let mut windows = Vec::with_capacity(sorted_intervals.len());
    let mut peak_windows = Vec::new();

    let mut max_cpu_some = 0.0_f64;
    let mut max_mem_some = 0.0_f64;
    let mut max_mem_full = 0.0_f64;
    let mut max_io_some = 0.0_f64;
    let mut max_io_full = 0.0_f64;

    let mut has_mem_psi = false;
    let mut has_io_psi = false;
    let mut has_near_spike_windows = false;

    for record in sorted_intervals {
        let near_spike = pressure_window_near_spike(record.elapsed_ms, clusters, cluster_window_ms);

        has_near_spike_windows |= near_spike;
        has_mem_psi |= record.mem_psi_some > 0.0 || record.mem_psi_full > 0.0;
        has_io_psi |= record.io_psi_some > 0.0 || record.io_psi_full > 0.0;

        max_cpu_some = max_cpu_some.max(record.cpu_psi_some);
        max_mem_some = max_mem_some.max(record.mem_psi_some);
        max_mem_full = max_mem_full.max(record.mem_psi_full);
        max_io_some = max_io_some.max(record.io_psi_some);
        max_io_full = max_io_full.max(record.io_psi_full);

        push_pressure_peak_window(
            &mut peak_windows,
            PressurePeakWindow {
                elapsed_ms: record.elapsed_ms,
                pressure_kind: PressureKind::CpuSome,
                value: record.cpu_psi_some,
                near_spike,
            },
        );
        push_pressure_peak_window(
            &mut peak_windows,
            PressurePeakWindow {
                elapsed_ms: record.elapsed_ms,
                pressure_kind: PressureKind::MemSome,
                value: record.mem_psi_some,
                near_spike,
            },
        );
        push_pressure_peak_window(
            &mut peak_windows,
            PressurePeakWindow {
                elapsed_ms: record.elapsed_ms,
                pressure_kind: PressureKind::MemFull,
                value: record.mem_psi_full,
                near_spike,
            },
        );
        push_pressure_peak_window(
            &mut peak_windows,
            PressurePeakWindow {
                elapsed_ms: record.elapsed_ms,
                pressure_kind: PressureKind::IoSome,
                value: record.io_psi_some,
                near_spike,
            },
        );
        push_pressure_peak_window(
            &mut peak_windows,
            PressurePeakWindow {
                elapsed_ms: record.elapsed_ms,
                pressure_kind: PressureKind::IoFull,
                value: record.io_psi_full,
                near_spike,
            },
        );

        windows.push(PressureWindow {
            elapsed_ms: record.elapsed_ms,
            cpu_some: record.cpu_psi_some,
            mem_some: Some(record.mem_psi_some),
            mem_full: Some(record.mem_psi_full),
            io_some: Some(record.io_psi_some),
            io_full: Some(record.io_psi_full),
            near_spike,
        });
    }

    peak_windows.sort_by(|a, b| {
        b.value
            .partial_cmp(&a.value)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.elapsed_ms.cmp(&b.elapsed_ms))
    });
    peak_windows.truncate(MAX_PRESSURE_PEAK_WINDOWS);

    let max_mem_some = has_mem_psi.then_some(max_mem_some);
    let max_mem_full = has_mem_psi.then_some(max_mem_full);
    let max_io_some = has_io_psi.then_some(max_io_some);
    let max_io_full = has_io_psi.then_some(max_io_full);

    let coverage = PressureTimelineCoverage {
        interval_records_loaded: windows.len(),
        has_cpu_psi: true,
        has_mem_psi,
        has_io_psi,
        has_near_spike_windows,
    };

    let pressure_notes = build_pressure_notes(PressureNoteInput {
        max_cpu_some,
        max_mem_some,
        max_mem_full,
        max_io_some,
        max_io_full,
        has_mem_psi,
        has_io_psi,
        peak_windows: &peak_windows,
    });

    PressureTimelineSummary {
        sample_count: windows.len(),
        max_cpu_some,
        max_mem_some,
        max_mem_full,
        max_io_some,
        max_io_full,
        windows,
        peak_windows,
        pressure_notes,
        coverage,
    }
}

pub(crate) fn pressure_window_near_spike(
    elapsed_ms: u64,
    clusters: &[SpikeCluster],
    cluster_window_ms: u64,
) -> bool {
    clusters.iter().any(|cluster| {
        cluster
            .points
            .iter()
            .filter_map(|point| point.elapsed_ms)
            .any(|cluster_elapsed_ms| elapsed_ms.abs_diff(cluster_elapsed_ms) <= cluster_window_ms)
    })
}

pub(crate) fn push_pressure_peak_window(
    peak_windows: &mut Vec<PressurePeakWindow>,
    peak_window: PressurePeakWindow,
) {
    if peak_window.value <= 0.0 {
        return;
    }

    peak_windows.push(peak_window);
}

pub(crate) struct PressureNoteInput<'a> {
    pub max_cpu_some: f64,
    pub max_mem_some: Option<f64>,
    pub max_mem_full: Option<f64>,
    pub max_io_some: Option<f64>,
    pub max_io_full: Option<f64>,
    pub has_mem_psi: bool,
    pub has_io_psi: bool,
    pub peak_windows: &'a [PressurePeakWindow],
}

pub(crate) fn build_pressure_notes(input: PressureNoteInput<'_>) -> Vec<String> {
    let max_cpu_some = input.max_cpu_some;
    let max_mem_some = input.max_mem_some;
    let max_mem_full = input.max_mem_full;
    let max_io_some = input.max_io_some;
    let max_io_full = input.max_io_full;
    let has_mem_psi = input.has_mem_psi;
    let has_io_psi = input.has_io_psi;
    let peak_windows = input.peak_windows;
    let mut notes = Vec::new();

    push_pressure_note_if_above(
        &mut notes,
        PressureKind::CpuSome,
        max_cpu_some,
        PRESSURE_NOTE_CPU_SOME,
        peak_windows,
        "CPU pressure",
    );

    if let Some(value) = max_mem_some {
        push_pressure_note_if_above(
            &mut notes,
            PressureKind::MemSome,
            value,
            PRESSURE_NOTE_MEM_SOME,
            peak_windows,
            "Memory pressure",
        );
    }
    if let Some(value) = max_mem_full {
        push_pressure_note_if_above(
            &mut notes,
            PressureKind::MemFull,
            value,
            PRESSURE_NOTE_MEM_FULL,
            peak_windows,
            "Memory full pressure",
        );
    }
    if let Some(value) = max_io_some {
        push_pressure_note_if_above(
            &mut notes,
            PressureKind::IoSome,
            value,
            PRESSURE_NOTE_IO_SOME,
            peak_windows,
            "I/O pressure",
        );
    }
    if let Some(value) = max_io_full {
        push_pressure_note_if_above(
            &mut notes,
            PressureKind::IoFull,
            value,
            PRESSURE_NOTE_IO_FULL,
            peak_windows,
            "I/O full pressure",
        );
    }

    if !has_mem_psi {
        notes.push("Memory PSI fields were not present in loaded intervals".to_owned());
    }
    if !has_io_psi {
        notes.push("I/O PSI fields were not present in loaded intervals".to_owned());
    }

    notes
}

pub(crate) fn push_pressure_note_if_above(
    notes: &mut Vec<String>,
    pressure_kind: PressureKind,
    value: f64,
    threshold: f64,
    peak_windows: &[PressurePeakWindow],
    label: &str,
) {
    if value < threshold {
        return;
    }

    let near_spike = peak_windows.iter().any(|peak_window| {
        pressure_kind_label(&peak_window.pressure_kind) == pressure_kind_label(&pressure_kind)
            && (peak_window.value - value).abs() <= f64::EPSILON
            && peak_window.near_spike
    });

    if near_spike {
        notes.push(format!(
            "{label} reached {:.1}% near a scheduler spike",
            value
        ));
    } else {
        notes.push(format!("{label} reached {:.1}%", value));
    }
}

pub(crate) fn pressure_kind_label(pressure_kind: &PressureKind) -> &'static str {
    match pressure_kind {
        PressureKind::CpuSome => "cpu_some",
        PressureKind::MemSome => "mem_some",
        PressureKind::MemFull => "mem_full",
        PressureKind::IoSome => "io_some",
        PressureKind::IoFull => "io_full",
    }
}

pub(crate) fn downgrade_quality(
    current: DataQualityLevel,
    candidate: DataQualityLevel,
) -> DataQualityLevel {
    use DataQualityLevel::{High, Low, Medium};

    match (current, candidate) {
        (Low, _) | (_, Low) => Low,
        (Medium, _) | (_, Medium) => Medium,
        (High, High) => High,
    }
}

pub(crate) fn spike_cluster_analysis(
    session: &SessionFile,
    spike_events: Option<&[SpikeEvent]>,
    cluster_window_ns: u64,
    top: usize,
    filter_class: Option<TaskClass>,
) -> SpikeClusterAnalysis {
    let (source, mut points) = match spike_events {
        Some(spike_events) => (
            SpikeClusterSource::SpikeEvents,
            flatten_spike_events(session, spike_events),
        ),
        None => (
            SpikeClusterSource::TopSpikesFallback,
            flatten_top_spikes(session),
        ),
    };

    if let Some(class) = filter_class {
        points.retain(|p| p.class == class);
    }

    let source_count = points.len();

    SpikeClusterAnalysis {
        source,
        source_count,
        clusters: spike_clusters_from_points(points, cluster_window_ns, top),
    }
}

pub(crate) fn spike_clusters_from_points(
    mut points: Vec<SpikePoint>,
    cluster_window_ns: u64,
    top: usize,
) -> Vec<SpikeCluster> {
    points.sort_by_key(|point| point.switch_ns);

    let mut candidates = BinaryHeap::new();
    let mut task_counts = BTreeMap::<u32, usize>::new();
    let mut max_latency_candidates = std::collections::VecDeque::<usize>::new();
    let mut left_idx = 0;

    for right_idx in 0..points.len() {
        *task_counts.entry(points[right_idx].task).or_default() += 1;

        while max_latency_candidates
            .back()
            .is_some_and(|idx| points[*idx].latency_ns <= points[right_idx].latency_ns)
        {
            max_latency_candidates.pop_back();
        }
        max_latency_candidates.push_back(right_idx);

        while left_idx <= right_idx
            && points[right_idx]
                .switch_ns
                .saturating_sub(points[left_idx].switch_ns)
                > cluster_window_ns
        {
            decrement_task_count(&mut task_counts, points[left_idx].task);
            if max_latency_candidates.front() == Some(&left_idx) {
                max_latency_candidates.pop_front();
            }
            left_idx += 1;
        }

        if task_counts.len() >= MIN_CLUSTER_TASKS {
            let max_latency_ns = max_latency_candidates
                .front()
                .map(|idx| points[*idx].latency_ns)
                .unwrap_or(0);

            let candidate = SpikeClusterCandidate {
                start_idx: left_idx,
                end_idx: right_idx + 1,
                distinct_tasks: task_counts.len(),
                min_switch_ns: points[left_idx].switch_ns,
                max_switch_ns: points[right_idx].switch_ns,
                max_latency_ns,
            };

            if candidates.len() < MAX_CLUSTER_CANDIDATES {
                candidates.push(std::cmp::Reverse(candidate));
            } else if let Some(mut worst) = candidates.peek_mut()
                && candidate > worst.0
            {
                *worst = std::cmp::Reverse(candidate);
            }
        }
    }

    let mut candidates_vec: Vec<_> = candidates.into_iter().map(|r| r.0).collect();
    candidates_vec.sort_by(|a, b| b.cmp(a));

    let mut selected_candidates = Vec::new();
    let max_selected = top.saturating_mul(4).min(MAX_CLUSTER_CANDIDATES);

    // Sweep-line: track selected intervals by max_switch_ns in a BTreeSet
    // for O(log n) overlap checking instead of O(n) per candidate.
    let mut selected_intervals: BTreeSet<(u64, u64)> = BTreeSet::new(); // (max_switch_ns, min_switch_ns)

    for candidate in candidates_vec {
        // Check for overlap: we need intervals where
        //   existing.min_switch_ns <= candidate.max_switch_ns AND
        //   existing.max_switch_ns >= candidate.min_switch_ns
        //
        // Since intervals are stored as (max_switch_ns, min_switch_ns),
        // we look for entries whose max_switch_ns >= candidate.min_switch_ns.
        let overlaps = selected_intervals
            .range((candidate.min_switch_ns, 0)..)
            .any(|(_, min_ns)| *min_ns <= candidate.max_switch_ns);

        if !overlaps {
            selected_intervals.insert((candidate.max_switch_ns, candidate.min_switch_ns));
            selected_candidates.push(candidate);
            if selected_candidates.len() >= max_selected {
                break;
            }
        }
    }

    selected_candidates
        .into_iter()
        .map(|candidate| {
            cluster_from_points(
                points[candidate.start_idx..candidate.end_idx].to_vec(),
                candidate.distinct_tasks,
            )
        })
        .collect()
}

pub(crate) fn flatten_spike_events(
    session: &SessionFile,
    spike_events: &[SpikeEvent],
) -> Vec<SpikePoint> {
    spike_events
        .iter()
        .map(|spike| SpikePoint {
            task: spike.task,
            class: spike.class,
            process_pid: spike.process_pid,
            comm: spike.comm.clone(),
            cpu: spike.cpu,
            wakeup_target_cpu: spike.wakeup_target_cpu,
            latency_ns: spike.latency_ns,
            wakeup_ns: spike.wakeup_ns,
            switch_ns: spike.switch_ns,
            target_pending_wakeups: spike.target_pending_wakeups,
            observed_runnable_depth: spike.observed_runnable_depth,
            switch_prev_pid: spike.switch_prev_pid,
            switch_prev_state: spike.switch_prev_state,
            switch_prev_state_label: classify_switch_prev_state(spike.switch_prev_state).to_owned(),
            elapsed_ms: elapsed_ms(session.core.monotonic_start_ns, spike.switch_ns)
                .or(spike.elapsed_ms),
            scx_ops: spike.scx_ops.clone(),
            scx_state: spike.scx_state.clone(),
            waker_tid: spike.waker_tid,
            waker_comm: spike.waker_comm.clone(),
            cause_tags: spike.cause_tags.clone(),
            primary_cause: spike.primary_cause.clone(),
        })
        .collect()
}

pub(crate) fn flatten_top_spikes(session: &SessionFile) -> Vec<SpikePoint> {
    let mut points = Vec::new();

    for task in &session.tasks {
        for spike in &task.top_spikes {
            points.push(spike_point_from_task(
                task,
                spike,
                elapsed_ms(session.core.monotonic_start_ns, spike.switch_ns),
            ));
        }
    }

    points
}

pub(crate) fn spike_point_from_task(
    task: &SessionTask,
    spike: &RecordedSpike,
    elapsed_ms: Option<u64>,
) -> SpikePoint {
    SpikePoint {
        task: task.task,
        class: spike.class,
        process_pid: spike.process_pid,
        comm: task.comm.clone(),
        cpu: spike.cpu,
        wakeup_target_cpu: spike.wakeup_target_cpu,
        switch_prev_pid: spike.switch_prev_pid,
        switch_prev_state: spike.switch_prev_state,
        switch_prev_state_label: classify_switch_prev_state(spike.switch_prev_state).to_owned(),
        latency_ns: spike.latency_ns,
        wakeup_ns: spike.wakeup_ns,
        switch_ns: spike.switch_ns,
        target_pending_wakeups: spike.target_pending_wakeups,
        observed_runnable_depth: spike.observed_runnable_depth,
        elapsed_ms,
        scx_ops: spike.scx_ops.clone(),
        scx_state: spike.scx_state.clone(),
        waker_tid: spike.waker_tid,
        waker_comm: spike.waker_comm.clone(),
        cause_tags: spike.cause_tags.clone(),
        primary_cause: spike.primary_cause.clone(),
    }
}

pub(crate) fn format_task_cpu_perf(task: &SessionTask) -> String {
    let Some(perf) = &task.cpu_perf else {
        return String::new();
    };

    let mut parts = Vec::new();
    if let Some(ipc) = perf.ipc {
        parts.push(format!("ipc={ipc:.2}"));
    }
    if let Some(cache_mpki) = perf.cache_mpki {
        parts.push(format!("cache_mpki={cache_mpki:.1}"));
    }
    if let Some(cache_miss_rate) = perf.cache_miss_rate {
        parts.push(format!("cache_miss_rate={:.1}%", cache_miss_rate * 100.0));
    }
    if let Some(cycles) = perf.cycles {
        parts.push(format!("cycles={cycles}"));
    }
    if let Some(instructions) = perf.instructions {
        parts.push(format!("instructions={instructions}"));
    }
    parts.push(format!("multiplexed={}", perf.multiplexed));

    format!(" cpu_perf: {}", parts.join(" "))
}

pub(crate) fn elapsed_ms(monotonic_start_ns: Option<u64>, switch_ns: u64) -> Option<u64> {
    let start_ns = monotonic_start_ns?;
    switch_ns
        .checked_sub(start_ns)
        .map(|elapsed_ns| elapsed_ns / 1_000_000)
}

pub(crate) fn decrement_task_count(task_counts: &mut BTreeMap<u32, usize>, task: u32) {
    let Some(count) = task_counts.get_mut(&task) else {
        return;
    };
    *count -= 1;
    if *count == 0 {
        task_counts.remove(&task);
    }
}

pub(crate) fn cluster_from_points(
    mut points: Vec<SpikePoint>,
    distinct_tasks: usize,
) -> SpikeCluster {
    points.sort_by_key(|point| (point.switch_ns, std::cmp::Reverse(point.latency_ns)));

    let min_switch_ns = points.first().map(|point| point.switch_ns).unwrap_or(0);
    let max_switch_ns = points.last().map(|point| point.switch_ns).unwrap_or(0);
    let max_latency_ns = points
        .iter()
        .map(|point| point.latency_ns)
        .max()
        .unwrap_or(0);

    let wake_graph = build_wake_graph(&points);

    SpikeCluster {
        points,
        distinct_tasks,
        min_switch_ns,
        max_switch_ns,
        max_latency_ns,
        diagnosis: None,
        diagnosis_explanation: None,
        anchor_task: None,
        anchor_class: None,
        anchor_comm: None,
        anchor_kind: None,
        foreground_pid: None,
        foreground_app_id: None,
        foreground_class: None,
        foreground_confidence: None,
        wake_graph,
    }
}

pub(crate) fn build_wake_graph(points: &[SpikePoint]) -> Vec<WakeGraphEdge> {
    let mut edges = BTreeMap::<(u32, String, u32, String), (u64, u64)>::new();

    for point in points {
        if point.waker_tid != 0 {
            let key = (
                point.waker_tid,
                point.waker_comm.clone(),
                point.task,
                point.comm.clone(),
            );
            let entry = edges.entry(key).or_insert((0, 0));
            entry.0 += 1;
            entry.1 = entry.1.max(point.latency_ns);
        }
    }

    let mut result: Vec<_> = edges
        .into_iter()
        .map(
            |((waker_tid, waker_comm, wakee_tid, wakee_comm), (count, max_latency_ns))| {
                WakeGraphEdge {
                    waker_tid,
                    waker_comm,
                    wakee_tid,
                    wakee_comm,
                    count,
                    max_latency_ns,
                }
            },
        )
        .collect();

    // Sort by count desc, then max_latency_ns desc
    result.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| b.max_latency_ns.cmp(&a.max_latency_ns))
    });

    result.truncate(16);
    result
}

pub(crate) fn perform_diagnosis(
    clusters: &mut [SpikeCluster],
    artifacts: &session_io::RunArtifacts,
    cluster_window_ns: u64,
) {
    for cluster in clusters {
        let diagnosis = diagnose_cluster(cluster, artifacts, cluster_window_ns);
        let diagnosis_explanation = explain_diagnosis(&diagnosis);
        let anchor = select_anchor_for_diagnosis(cluster, &diagnosis);
        cluster.anchor_task = Some(anchor.task);
        cluster.anchor_class = Some(anchor.class);
        cluster.anchor_comm = Some(anchor.comm);
        cluster.anchor_kind = Some(anchor.kind);
        cluster.diagnosis_explanation = Some(diagnosis_explanation);
        cluster.diagnosis = Some(diagnosis);
    }
}

pub(crate) fn explain_diagnosis(diagnosis: &Diagnosis) -> DiagnosisExplanation {
    let primary = diagnosis.primary.as_ref();
    let evidence_items = primary
        .map(|primary| {
            primary
                .evidence
                .iter()
                .map(|evidence| DiagnosisEvidenceView {
                    kind: format!("{:?}", evidence.kind),
                    strength: evidence.strength,
                    message: evidence.message.clone(),
                    timestamp_ms: evidence.timestamp_ms,
                })
                .collect()
        })
        .unwrap_or_default();

    let competing_candidates = diagnosis
        .candidates
        .iter()
        .skip(usize::from(primary.is_some()))
        .map(|candidate| DiagnosisCandidateView {
            cause: format!("{:?}", candidate.cause),
            score: candidate.score,
            confidence: format!("{:?}", candidate.confidence),
            evidence_count: candidate.evidence.len(),
        })
        .collect();

    DiagnosisExplanation {
        primary_cause: primary.map(|primary| format!("{:?}", primary.cause)),
        primary_score: primary.map(|primary| primary.score),
        primary_confidence: primary.map(|primary| format!("{:?}", primary.confidence)),
        reason: diagnosis.summary.clone(),
        evidence_items,
        competing_candidates,
        missing_evidence: diagnosis.missing_evidence.clone(),
    }
}

pub(crate) fn annotate_clusters_with_foreground(
    clusters: &mut [SpikeCluster],
    foreground_events: &[ForegroundEvent],
    max_stale_ms: u64,
) {
    for cluster in clusters {
        if let Some(event) = foreground_for_cluster(cluster, foreground_events, max_stale_ms) {
            cluster.foreground_pid = event.pid;
            cluster.foreground_app_id = event.app_id.clone();
            cluster.foreground_class = event.class.clone();
            cluster.foreground_confidence = Some(event.confidence);
        }
    }
}

pub(crate) fn foreground_for_cluster<'a>(
    cluster: &SpikeCluster,
    foreground_events: &'a [ForegroundEvent],
    max_stale_ms: u64,
) -> Option<&'a ForegroundEvent> {
    foreground_for_elapsed_ms(
        cluster_elapsed_ms(cluster)?,
        foreground_events,
        max_stale_ms,
    )
}

pub(crate) fn foreground_for_elapsed_ms(
    elapsed_ms: u64,
    foreground_events: &[ForegroundEvent],
    max_stale_ms: u64,
) -> Option<&ForegroundEvent> {
    foreground_events
        .iter()
        .filter(|event| event.elapsed_ms <= elapsed_ms)
        .filter(|event| elapsed_ms.saturating_sub(event.elapsed_ms) <= max_stale_ms)
        .max_by_key(|event| event.elapsed_ms)
}

pub(crate) fn cluster_elapsed_ms(cluster: &SpikeCluster) -> Option<u64> {
    cluster
        .points
        .iter()
        .filter_map(|point| point.elapsed_ms)
        .min()
}

pub(crate) fn cluster_elapsed_range(cluster: &SpikeCluster) -> Option<(u64, u64)> {
    let mut elapsed = cluster.points.iter().filter_map(|point| point.elapsed_ms);
    let first = elapsed.next()?;
    let mut min_elapsed = first;
    let mut max_elapsed = first;
    for value in elapsed {
        min_elapsed = min_elapsed.min(value);
        max_elapsed = max_elapsed.max(value);
    }
    Some((min_elapsed, max_elapsed))
}

pub(crate) fn format_option<T: std::fmt::Display>(value: Option<T>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_owned())
}

pub(crate) fn format_process_pid(process_pid: Option<u32>) -> String {
    match process_pid {
        Some(process_pid) => process_pid.to_string(),
        None => "-".to_owned(),
    }
}

pub(crate) fn cluster_elapsed(cluster: &SpikeCluster) -> Option<u64> {
    cluster
        .points
        .iter()
        .filter_map(|point| point.elapsed_ms)
        .min()
}

pub(crate) fn format_elapsed(elapsed_ms: Option<u64>) -> String {
    match elapsed_ms {
        Some(elapsed_ms) => format!("{elapsed_ms}ms"),
        None => "-".to_owned(),
    }
}

pub(crate) fn cluster_labels(cluster: &SpikeCluster) -> Vec<&'static str> {
    let mut labels = Vec::new();

    if cluster
        .points
        .iter()
        .any(|point| point.comm == "RenderThread" || point.comm == "Main")
    {
        labels.push("render-main");
    }

    if cluster
        .points
        .iter()
        .any(|point| point.comm.starts_with("dxvk-"))
    {
        labels.push("dxvk");
    }

    if cluster
        .points
        .iter()
        .any(|point| point.comm == "wineserver" || point.comm.contains("winedevice"))
    {
        labels.push("wine");
    }

    if cluster
        .points
        .iter()
        .any(|point| point.comm == "AudioThread")
    {
        labels.push("audio");
    }

    labels
}

pub(crate) fn percentile_warning_note(percentile_scope: &str) -> &'static str {
    match percentile_scope {
        "histogram" => {
            "p95/p99 are approximate histogram estimates across the full session; max and threshold counters are exact"
        }
        "capped_prefix" | "capped" => {
            "p95/p99 are capped prefix estimates; prefer max and over_1ms/over_2ms/over_5ms"
        }
        _ => {
            "p95/p99 may be capped because this session predates histogram percentiles; prefer max and threshold counters"
        }
    }
}

pub(crate) fn calculate_median_frametime(frames: &[FrameEvent]) -> f64 {
    if frames.is_empty() {
        return 0.0;
    }
    let mut times: Vec<_> = frames.iter().map(|f| f.frametime_ms).collect();
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
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
            foreground_pid: foreground.and_then(|event| event.pid),
            foreground_app_id: foreground.and_then(|event| event.app_id.clone()),
            foreground_class: foreground.and_then(|event| event.class.clone()),
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

pub(crate) fn median_f64(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }

    values.sort_by(|a, b| a.total_cmp(b));
    let mid = values.len() / 2;
    if values.len().is_multiple_of(2) {
        (values[mid - 1] + values[mid]) / 2.0
    } else {
        values[mid]
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
