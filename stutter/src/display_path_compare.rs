use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::{
    artifacts::ArtifactSelection,
    process_tree::TaskClass,
    recorder::{DrmFenceEventRecord, KmsFlipEventRecord},
    report::ReportAnalysisJson,
    session_io,
};

#[derive(Debug, Clone)]
pub struct DisplayPathCompareInput {
    pub baseline: PathBuf,
    pub test: PathBuf,
    pub json: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DisplayPathCompareOutput {
    pub display_path_cost: DisplayPathCostSummary,
}

#[derive(Debug, Clone, Serialize)]
pub struct DisplayPathCostSummary {
    pub baseline_label: Option<String>,
    pub test_label: Option<String>,
    pub comparison_quality: String,
    pub comparison_warnings: Vec<String>,
    pub baseline_fps: Option<f64>,
    pub test_fps: Option<f64>,
    pub avg_fps_delta: Option<f64>,
    pub avg_fps_delta_percent: Option<f64>,
    pub median_frame_delta_ms: Option<f64>,
    pub p95_frame_delta_ms: Option<f64>,
    pub p99_frame_delta_ms: Option<f64>,
    pub max_frame_delta_ms: Option<f64>,
    pub kms_median_delta_ms: Option<f64>,
    pub kms_p95_delta_ms: Option<f64>,
    pub kms_p99_delta_ms: Option<f64>,
    pub kms_long_flip_delta_count: Option<i64>,
    pub display_side_fence_wait_p99_delta_ms: Option<f64>,
    pub render_side_fence_wait_p99_delta_ms: Option<f64>,
    pub cross_gpu_candidate_count_delta: i64,
    pub commit_to_present_p99_delta_ms: Option<f64>,
    pub discarded_frame_delta: i64,
    pub zero_copy_ratio_delta: Option<f64>,
    pub game_cluster_count_delta: i64,
    pub compositor_cluster_count_delta: i64,
    pub scheduler_p99_delta_ms: Option<f64>,
    pub likely_causes: Vec<String>,
    pub notes: Vec<String>,
}

pub fn run_display_path_compare(input: DisplayPathCompareInput) -> anyhow::Result<()> {
    let output = compare_display_path(&input.baseline, &input.test)?;
    if input.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        print_display_path_compare(&output.display_path_cost);
    }
    Ok(())
}

pub fn compare_display_path(
    baseline_path: &Path,
    test_path: &Path,
) -> anyhow::Result<DisplayPathCompareOutput> {
    let baseline = crate::report::build_report_analysis(baseline_path, 10, 5, None)?;
    let test = crate::report::build_report_analysis(test_path, 10, 5, None)?;
    let baseline_artifacts =
        session_io::load_run_artifacts(baseline_path, ArtifactSelection::report())?;
    let test_artifacts = session_io::load_run_artifacts(test_path, ArtifactSelection::report())?;

    let mut warnings = Vec::new();
    let mut max_severity = 0_u8;
    validate_comparability(&baseline, &test, &mut warnings, &mut max_severity);
    validate_probe_match(
        "KMS timing",
        baseline.session.core.kms_flip_event_count,
        test.session.core.kms_flip_event_count,
        &mut warnings,
        &mut max_severity,
    );
    validate_probe_match(
        "DRM fence latency",
        baseline.session.core.drm_fence_event_count,
        test.session.core.drm_fence_event_count,
        &mut warnings,
        &mut max_severity,
    );
    validate_probe_match(
        "Wayland presentation",
        baseline.session.core.wayland_presentation_event_count,
        test.session.core.wayland_presentation_event_count,
        &mut warnings,
        &mut max_severity,
    );

    let baseline_fps = fps(&baseline);
    let test_fps = fps(&test);
    let avg_fps_delta = delta(test_fps, baseline_fps);
    let avg_fps_delta_percent = avg_fps_delta
        .zip(baseline_fps)
        .and_then(|(delta, baseline)| (baseline != 0.0).then_some(delta / baseline * 100.0));

    let kms_long_flip_delta_count = Some(
        count_long_kms_flips(&test_artifacts.kms_flip_events) as i64
            - count_long_kms_flips(&baseline_artifacts.kms_flip_events) as i64,
    );
    let display_side_fence_wait_p99_delta_ms = delta(
        role_fence_p99_ms(&test_artifacts.drm_fence_events, "display"),
        role_fence_p99_ms(&baseline_artifacts.drm_fence_events, "display"),
    );
    let render_side_fence_wait_p99_delta_ms = delta(
        role_fence_p99_ms(&test_artifacts.drm_fence_events, "render"),
        role_fence_p99_ms(&baseline_artifacts.drm_fence_events, "render"),
    );
    let discarded_frame_delta = test.wayland_presentation.discarded_count as i64
        - baseline.wayland_presentation.discarded_count as i64;
    let zero_copy_ratio_delta = delta(
        test.wayland_presentation.zero_copy_ratio,
        baseline.wayland_presentation.zero_copy_ratio,
    );

    let mut likely_causes = Vec::new();
    if display_side_fence_wait_p99_delta_ms.is_some_and(|value| value > 1.0)
        || (test.drm_fence_timing.cross_gpu_candidate_count as i64
            - baseline.drm_fence_timing.cross_gpu_candidate_count as i64)
            > 0
    {
        likely_causes.push("cross_gpu_fence_wait_candidate".to_owned());
    }
    if test
        .wayland_presentation
        .compositor_queue_candidate_count
        .saturating_sub(
            baseline
                .wayland_presentation
                .compositor_queue_candidate_count,
        )
        > 0
        || test
            .wayland_presentation
            .p99_commit_to_present_ms
            .zip(baseline.wayland_presentation.p99_commit_to_present_ms)
            .is_some_and(|(test, baseline)| test - baseline > 1.0)
    {
        likely_causes.push("wayland_presentation_queue_candidate".to_owned());
    }
    if test
        .kms_timing
        .p99_flip_ms
        .zip(baseline.kms_timing.p99_flip_ms)
        .is_some_and(|(test, baseline)| test - baseline > 1.0)
    {
        likely_causes.push("scanout_kms_pageflip_candidate".to_owned());
    }

    let notes = vec![
        "This is an A/B estimate, not direct photon latency.".to_owned(),
        "Input-to-photon latency requires external measurement hardware.".to_owned(),
        "If scheduler latency worsened too, do not attribute the delta to display path alone."
            .to_owned(),
    ];

    Ok(DisplayPathCompareOutput {
        display_path_cost: DisplayPathCostSummary {
            baseline_label: display_path_label(&baseline),
            test_label: display_path_label(&test),
            comparison_quality: match max_severity {
                0 if warnings.is_empty() => "high",
                2.. => "low",
                _ => "medium",
            }
            .to_owned(),
            comparison_warnings: warnings,
            baseline_fps,
            test_fps,
            avg_fps_delta,
            avg_fps_delta_percent,
            median_frame_delta_ms: delta(
                test.frame_pacing.median_frametime_ms,
                baseline.frame_pacing.median_frametime_ms,
            ),
            p95_frame_delta_ms: delta(
                test.frame_pacing.p95_frametime_ms,
                baseline.frame_pacing.p95_frametime_ms,
            ),
            p99_frame_delta_ms: delta(
                test.frame_pacing.p99_frametime_ms,
                baseline.frame_pacing.p99_frametime_ms,
            ),
            max_frame_delta_ms: delta(
                test.frame_pacing.max_frametime_ms,
                baseline.frame_pacing.max_frametime_ms,
            ),
            kms_median_delta_ms: delta(
                test.kms_timing.median_flip_ms,
                baseline.kms_timing.median_flip_ms,
            ),
            kms_p95_delta_ms: delta(test.kms_timing.p95_flip_ms, baseline.kms_timing.p95_flip_ms),
            kms_p99_delta_ms: delta(test.kms_timing.p99_flip_ms, baseline.kms_timing.p99_flip_ms),
            kms_long_flip_delta_count,
            display_side_fence_wait_p99_delta_ms,
            render_side_fence_wait_p99_delta_ms,
            cross_gpu_candidate_count_delta: test.drm_fence_timing.cross_gpu_candidate_count as i64
                - baseline.drm_fence_timing.cross_gpu_candidate_count as i64,
            commit_to_present_p99_delta_ms: delta(
                test.wayland_presentation.p99_commit_to_present_ms,
                baseline.wayland_presentation.p99_commit_to_present_ms,
            ),
            discarded_frame_delta,
            zero_copy_ratio_delta,
            game_cluster_count_delta: test.frame_pacing.game_cluster_count as i64
                - baseline.frame_pacing.game_cluster_count as i64,
            compositor_cluster_count_delta: test.frame_pacing.compositor_cluster_count as i64
                - baseline.frame_pacing.compositor_cluster_count as i64,
            scheduler_p99_delta_ms: delta(scheduler_p99_ms(&test), scheduler_p99_ms(&baseline)),
            likely_causes,
            notes,
        },
    })
}

fn print_display_path_compare(summary: &DisplayPathCostSummary) {
    println!("Estimated display-path cost:");
    println!(
        "  labels:          {} -> {}",
        summary.baseline_label.as_deref().unwrap_or("baseline"),
        summary.test_label.as_deref().unwrap_or("test")
    );
    println!(
        "  FPS:             {}",
        format_optional_percent(summary.avg_fps_delta_percent)
    );
    println!(
        "  median frame:    {}",
        format_optional_ms(summary.median_frame_delta_ms)
    );
    println!(
        "  p95 frame:       {}",
        format_optional_ms(summary.p95_frame_delta_ms)
    );
    println!(
        "  p99 frame:       {}",
        format_optional_ms(summary.p99_frame_delta_ms)
    );
    println!(
        "  KMS p99:         {}",
        format_optional_ms(summary.kms_p99_delta_ms)
    );
    println!(
        "  fence p99:       {}",
        format_optional_ms(summary.display_side_fence_wait_p99_delta_ms)
    );
    println!(
        "  Wayland p99:     {}",
        format_optional_ms(summary.commit_to_present_p99_delta_ms)
    );
    println!("Confidence:");
    println!("  {}", summary.comparison_quality);
    if !summary.likely_causes.is_empty() {
        println!("Likely cause:");
        for cause in &summary.likely_causes {
            println!("  {cause}");
        }
    }
    for warning in &summary.comparison_warnings {
        println!("warning: {warning}");
    }
    for note in &summary.notes {
        println!("note: {note}");
    }
}

fn validate_comparability(
    baseline: &ReportAnalysisJson,
    test: &ReportAnalysisJson,
    warnings: &mut Vec<String>,
    max_severity: &mut u8,
) {
    if baseline.session.core.run_name.is_some()
        && test.session.core.run_name.is_some()
        && baseline.session.core.run_name != test.session.core.run_name
    {
        warn(
            warnings,
            max_severity,
            2,
            "different scenario/run names; comparison may not isolate display path",
        );
    }
    let baseline_duration = baseline.session.core.duration_ms.max(1) as f64;
    let test_duration = test.session.core.duration_ms.max(1) as f64;
    let duration_delta = ((test_duration - baseline_duration) / baseline_duration).abs();
    if duration_delta > 0.20 {
        warn(
            warnings,
            max_severity,
            2,
            "durations differ by more than 20%",
        );
    } else if duration_delta > 0.10 {
        warn(
            warnings,
            max_severity,
            1,
            "durations differ by more than 10%",
        );
    }
    if baseline.frame_pacing.frame_count == 0 || test.frame_pacing.frame_count == 0 {
        warn(
            warnings,
            max_severity,
            2,
            "one or both runs lack frame events",
        );
    }
    if top_task_class(baseline) != top_task_class(test) {
        warn(
            warnings,
            max_severity,
            1,
            "top task class differs between runs",
        );
    }
    if baseline.data_quality.level != crate::report::DataQualityLevel::High
        || test.data_quality.level != crate::report::DataQualityLevel::High
    {
        warn(
            warnings,
            max_severity,
            1,
            "one or both reports have non-high data quality",
        );
    }
}

fn validate_probe_match(
    label: &str,
    baseline_count: u64,
    test_count: u64,
    warnings: &mut Vec<String>,
    max_severity: &mut u8,
) {
    if (baseline_count == 0) != (test_count == 0) {
        warn(
            warnings,
            max_severity,
            1,
            format!("{label} availability differs between runs"),
        );
    }
}

fn warn(
    warnings: &mut Vec<String>,
    max_severity: &mut u8,
    severity: u8,
    message: impl Into<String>,
) {
    *max_severity = (*max_severity).max(severity);
    warnings.push(message.into());
}

fn display_path_label(analysis: &ReportAnalysisJson) -> Option<String> {
    analysis
        .session
        .core
        .display_path
        .as_ref()
        .and_then(|display| display.label.clone())
        .or_else(|| analysis.session.config.display_path_label.clone())
}

fn fps(analysis: &ReportAnalysisJson) -> Option<f64> {
    (analysis.frame_pacing.frame_count > 0 && analysis.session.core.duration_ms > 0).then_some(
        analysis.frame_pacing.frame_count as f64
            / (analysis.session.core.duration_ms as f64 / 1000.0),
    )
}

fn delta(test: Option<f64>, baseline: Option<f64>) -> Option<f64> {
    test.zip(baseline).map(|(test, baseline)| test - baseline)
}

fn count_long_kms_flips(events: &[KmsFlipEventRecord]) -> usize {
    events
        .iter()
        .filter(|event| {
            event
                .duration_ns
                .is_some_and(|duration| duration >= 1_000_000)
        })
        .count()
}

fn role_fence_p99_ms(events: &[DrmFenceEventRecord], role: &str) -> Option<f64> {
    let mut values = events
        .iter()
        .filter(|event| event.gpu_role.as_deref() == Some(role))
        .filter_map(|event| event.duration_ns)
        .map(|duration| duration as f64 / 1_000_000.0)
        .collect::<Vec<_>>();
    percentile(&mut values, 0.99)
}

fn scheduler_p99_ms(analysis: &ReportAnalysisJson) -> Option<f64> {
    analysis
        .session
        .tasks
        .iter()
        .map(|task| task.latency.p99_ns as f64 / 1_000_000.0)
        .reduce(f64::max)
}

fn top_task_class(analysis: &ReportAnalysisJson) -> Option<TaskClass> {
    analysis.session.tasks.first().map(|task| task.class)
}

fn percentile(values: &mut [f64], percentile: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((values.len() - 1) as f64 * percentile).round() as usize;
    values.get(idx).copied()
}

fn format_optional_ms(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:+.1} ms"))
        .unwrap_or_else(|| "n/a".to_owned())
}

fn format_optional_percent(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:+.1}%"))
        .unwrap_or_else(|| "n/a".to_owned())
}
