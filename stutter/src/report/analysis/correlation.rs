use std::collections::BTreeSet;

use super::*;

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
                let Some(elapsed) = cluster_elapsed(cluster) else {
                    return Vec::new();
                };
                let Some(sample) = matches
                    .iter()
                    .min_by_key(|sample| sample.elapsed_ms.abs_diff(elapsed))
                else {
                    return Vec::new();
                };
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
                let Some((min_elapsed, max_elapsed)) = cluster_elapsed_range(cluster) else {
                    return Vec::new();
                };
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
                let Some(elapsed) = cluster_elapsed(cluster) else {
                    return Vec::new();
                };
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
