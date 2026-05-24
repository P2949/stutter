use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::{
    artifacts::ArtifactSelection,
    display_topology::{ConnectorInfo, DisplayTopologySnapshot},
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
    pub strict: bool,
    pub expect: Option<DisplayPathExpectation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
#[value(rename_all = "kebab-case")]
pub enum DisplayPathExpectation {
    DirectToOffload,
    OffloadToDirect,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
pub struct DisplayPathCompareOutput {
    pub display_path_cost: DisplayPathCostSummary,
}

#[derive(Debug, Clone, Serialize)]
pub struct DisplayPathCostSummary {
    pub verdict: String,
    pub confidence_score: f64,
    pub evidence: Vec<String>,
    pub missing_evidence: Vec<String>,
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
    pub direct_scanout_status_delta: Option<String>,
    pub igpu_engine_activity_delta: Option<f64>,
    pub dmabuf_copy_required_delta: Option<i64>,
    pub fence_component_delta_ms: Option<f64>,
    pub kms_component_delta_ms: Option<f64>,
    pub wayland_component_delta_ms: Option<f64>,
    pub compositor_component_delta_ms: Option<f64>,
    pub game_cluster_count_delta: i64,
    pub compositor_cluster_count_delta: i64,
    pub scheduler_p99_delta_ms: Option<f64>,
    pub likely_causes: Vec<String>,
    pub notes: Vec<String>,
}

pub fn run_display_path_compare(input: DisplayPathCompareInput) -> anyhow::Result<()> {
    let output = compare_display_path_with_options(
        &input.baseline,
        &input.test,
        input.expect,
        input.strict,
    )?;
    if input.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        print_display_path_compare(&output.display_path_cost);
    }
    Ok(())
}

pub fn compare_display_path_with_options(
    baseline_path: &Path,
    test_path: &Path,
    expect: Option<DisplayPathExpectation>,
    strict: bool,
) -> anyhow::Result<DisplayPathCompareOutput> {
    let baseline = crate::report::build_report_analysis(baseline_path, 10, 5, None)?;
    let test = crate::report::build_report_analysis(test_path, 10, 5, None)?;
    let baseline_artifacts =
        session_io::load_run_artifacts(baseline_path, ArtifactSelection::report())?;
    let test_artifacts = session_io::load_run_artifacts(test_path, ArtifactSelection::report())?;

    let mut warnings = Vec::new();
    let mut max_severity = 0_u8;
    validate_comparability(&baseline, &test, &mut warnings, &mut max_severity);
    validate_topology_match(
        baseline_artifacts.display_topology.as_ref(),
        test_artifacts.display_topology.as_ref(),
        &mut warnings,
        &mut max_severity,
    );
    validate_display_path_expectation(expect, &baseline, &test, &mut warnings, &mut max_severity);
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
    validate_probe_match(
        "DMABUF path",
        baseline.session.core.dmabuf_event_count,
        test.session.core.dmabuf_event_count,
        &mut warnings,
        &mut max_severity,
    );
    validate_probe_match(
        "GPU engine sampling",
        baseline.session.core.gpu_engine_sample_count,
        test.session.core.gpu_engine_sample_count,
        &mut warnings,
        &mut max_severity,
    );
    if strict && max_severity >= 2 {
        warnings.push(
            "strict comparison failed: high-severity comparability warnings present".to_owned(),
        );
    }

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
    let direct_scanout_status_delta =
        (baseline.direct_scanout.status != test.direct_scanout.status).then(|| {
            format!(
                "{} -> {}",
                baseline.direct_scanout.status, test.direct_scanout.status
            )
        });
    let baseline_igpu_activity = igpu_engine_activity(&baseline);
    let test_igpu_activity = igpu_engine_activity(&test);
    let igpu_engine_activity_delta = Some(test_igpu_activity - baseline_igpu_activity);
    let dmabuf_copy_required_delta = Some(
        test.dmabuf_path.copy_required_count as i64
            - baseline.dmabuf_path.copy_required_count as i64,
    );
    let fence_component_delta_ms = display_side_fence_wait_p99_delta_ms;
    let kms_component_delta_ms =
        delta(test.kms_timing.p99_flip_ms, baseline.kms_timing.p99_flip_ms);
    let wayland_component_delta_ms = delta(
        test.wayland_presentation.p99_commit_to_present_ms,
        baseline.wayland_presentation.p99_commit_to_present_ms,
    );
    let compositor_component_delta_ms = delta(
        Some(test.display_path_diagnosis.compositor_component.score),
        Some(baseline.display_path_diagnosis.compositor_component.score),
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
        || wayland_component_delta_ms.is_some_and(|value| value > 1.0)
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
    if igpu_engine_activity_delta.is_some_and(|value| value > 0.0) {
        likely_causes.push("igpu_blitter_activity_near_outliers".to_owned());
    }
    if dmabuf_copy_required_delta.is_some_and(|value| value > 0) {
        likely_causes.push("dmabuf_copy_required_candidate".to_owned());
    }

    let mut evidence = compare_evidence(
        &baseline,
        &test,
        CompareEvidenceDeltas {
            avg_fps_delta_percent,
            fence_delta_ms: display_side_fence_wait_p99_delta_ms,
            kms_delta_ms: kms_component_delta_ms,
            wayland_delta_ms: wayland_component_delta_ms,
            igpu_delta: igpu_engine_activity_delta,
            dmabuf_copy_delta: dmabuf_copy_required_delta,
        },
    );
    evidence.extend(test.display_path_diagnosis.evidence.iter().take(6).cloned());
    evidence.sort();
    evidence.dedup();

    let mut missing_evidence = baseline.display_path_diagnosis.missing_evidence.clone();
    missing_evidence.extend(test.display_path_diagnosis.missing_evidence.iter().cloned());
    missing_evidence.extend(
        warnings
            .iter()
            .filter(|warning| warning.contains("availability differs"))
            .cloned(),
    );
    missing_evidence.sort();
    missing_evidence.dedup();
    let verdict = compare_verdict(expect, avg_fps_delta_percent, &baseline, &test);
    let confidence_score = comparison_confidence_score(max_severity, evidence.len(), strict);

    let notes = vec![
        "This is an A/B estimate, not direct photon latency.".to_owned(),
        "Input-to-photon latency requires external measurement hardware.".to_owned(),
        "If scheduler latency worsened too, do not attribute the delta to display path alone."
            .to_owned(),
    ];

    Ok(DisplayPathCompareOutput {
        display_path_cost: DisplayPathCostSummary {
            verdict,
            confidence_score,
            evidence,
            missing_evidence,
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
            direct_scanout_status_delta,
            igpu_engine_activity_delta,
            dmabuf_copy_required_delta,
            fence_component_delta_ms,
            kms_component_delta_ms,
            wayland_component_delta_ms,
            compositor_component_delta_ms,
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
    println!("Display-path A/B verdict:");
    println!("  {}", summary.verdict);
    println!("  confidence_score: {:.2}", summary.confidence_score);
    println!();
    println!("Measured cost:");
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
    println!(
        "  iGPU activity:   {}",
        format_optional_count_delta(summary.igpu_engine_activity_delta)
    );
    println!(
        "  DMABUF copies:   {}",
        format_optional_i64(summary.dmabuf_copy_required_delta)
    );
    println!("Comparison quality: {}", summary.comparison_quality);
    if !summary.likely_causes.is_empty() {
        println!("Likely components:");
        for cause in &summary.likely_causes {
            println!("  - {cause}");
        }
    }
    if !summary.evidence.is_empty() {
        println!("Evidence:");
        for evidence in &summary.evidence {
            println!("  - {evidence}");
        }
    }
    if !summary.missing_evidence.is_empty() {
        println!("Missing evidence:");
        for missing in &summary.missing_evidence {
            println!("  - {missing}");
        }
    }
    for warning in &summary.comparison_warnings {
        println!("warning: {warning}");
    }
    for note in &summary.notes {
        println!("note: {note}");
    }
}

fn validate_display_path_expectation(
    expect: Option<DisplayPathExpectation>,
    baseline: &ReportAnalysisJson,
    test: &ReportAnalysisJson,
    warnings: &mut Vec<String>,
    max_severity: &mut u8,
) {
    let Some(expect) = expect else {
        return;
    };
    match expect {
        DisplayPathExpectation::DirectToOffload => {
            if same_scanout_gpu(baseline, test) {
                warn(
                    warnings,
                    max_severity,
                    2,
                    "expected direct-to-offload but baseline and test scanout GPU did not differ",
                );
            }
            if test.display_path_diagnosis.is_cross_gpu != Some(true) {
                warn(
                    warnings,
                    max_severity,
                    2,
                    "expected direct-to-offload but test run was not identified as cross-GPU",
                );
            }
        }
        DisplayPathExpectation::OffloadToDirect => {
            if same_scanout_gpu(baseline, test) {
                warn(
                    warnings,
                    max_severity,
                    2,
                    "expected offload-to-direct but baseline and test scanout GPU did not differ",
                );
            }
            if baseline.display_path_diagnosis.is_cross_gpu != Some(true) {
                warn(
                    warnings,
                    max_severity,
                    2,
                    "expected offload-to-direct but baseline run was not identified as cross-GPU",
                );
            }
        }
        DisplayPathExpectation::Unknown => {}
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
    if top_process_comm(baseline) != top_process_comm(test) {
        warn(
            warnings,
            max_severity,
            1,
            "top process differs between runs",
        );
    }
    let frame_delta = rough_count_delta(
        baseline.frame_pacing.frame_count,
        test.frame_pacing.frame_count,
    );
    if frame_delta > 0.25 {
        warn(
            warnings,
            max_severity,
            2,
            "frame counts differ by more than 25%",
        );
    } else if frame_delta > 0.15 {
        warn(
            warnings,
            max_severity,
            1,
            "frame counts differ by more than 15%",
        );
    }
    if display_session_type(baseline) != display_session_type(test) {
        warn(
            warnings,
            max_severity,
            1,
            "session type differs between runs",
        );
    }
    if display_compositor(baseline) != display_compositor(test) {
        warn(warnings, max_severity, 1, "compositor differs between runs");
    }
    if display_render_driver(baseline) != display_render_driver(test) {
        warn(
            warnings,
            max_severity,
            2,
            "comparison downgraded: render GPU changed",
        );
    }
    if display_connector(baseline) != display_connector(test) {
        warn(warnings, max_severity, 1, "connector differs between runs");
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

fn validate_topology_match(
    baseline: Option<&DisplayTopologySnapshot>,
    test: Option<&DisplayTopologySnapshot>,
    warnings: &mut Vec<String>,
    max_severity: &mut u8,
) {
    let Some(baseline) = baseline else {
        return;
    };
    let Some(test) = test else {
        return;
    };
    let baseline_connector = selected_connector(baseline);
    let test_connector = selected_connector(test);
    if baseline_connector
        .zip(test_connector)
        .is_some_and(|(baseline, test)| baseline.edid_hash != test.edid_hash)
    {
        warn(
            warnings,
            max_severity,
            1,
            "comparison downgraded: connected display EDID differs",
        );
    }
    if baseline_connector
        .zip(test_connector)
        .is_some_and(|(baseline, test)| baseline.modes.first() != test.modes.first())
    {
        warn(
            warnings,
            max_severity,
            1,
            "comparison downgraded: test and baseline used different refresh modes",
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

struct CompareEvidenceDeltas {
    avg_fps_delta_percent: Option<f64>,
    fence_delta_ms: Option<f64>,
    kms_delta_ms: Option<f64>,
    wayland_delta_ms: Option<f64>,
    igpu_delta: Option<f64>,
    dmabuf_copy_delta: Option<i64>,
}

fn compare_evidence(
    baseline: &ReportAnalysisJson,
    test: &ReportAnalysisJson,
    deltas: CompareEvidenceDeltas,
) -> Vec<String> {
    let mut evidence = Vec::new();
    if let Some(delta) = deltas.avg_fps_delta_percent {
        evidence.push(format!("avg FPS delta: {delta:+.1}%"));
    }
    if let Some(delta) = deltas.fence_delta_ms {
        evidence.push(format!("display fence p99 delta: {delta:+.2} ms"));
    }
    if let Some(delta) = deltas.kms_delta_ms {
        evidence.push(format!("KMS p99 delta: {delta:+.2} ms"));
    }
    if let Some(delta) = deltas.wayland_delta_ms {
        evidence.push(format!(
            "Wayland commit-to-present p99 delta: {delta:+.2} ms"
        ));
    }
    if let Some(delta) = deltas.igpu_delta
        && delta != 0.0
    {
        evidence.push(format!(
            "iGPU render/blitter activity delta: {delta:+.0} samples"
        ));
    }
    if let Some(delta) = deltas.dmabuf_copy_delta
        && delta != 0
    {
        evidence.push(format!("DMABUF copy-required delta: {delta:+}"));
    }
    let suspicion_delta = test.display_path_diagnosis.suspicion_score
        - baseline.display_path_diagnosis.suspicion_score;
    if suspicion_delta.abs() >= 0.05 {
        evidence.push(format!(
            "display-path suspicion score delta: {suspicion_delta:+.2}"
        ));
    }
    evidence
}

fn compare_verdict(
    expect: Option<DisplayPathExpectation>,
    avg_fps_delta_percent: Option<f64>,
    baseline: &ReportAnalysisJson,
    test: &ReportAnalysisJson,
) -> String {
    let suspicion_delta = test.display_path_diagnosis.suspicion_score
        - baseline.display_path_diagnosis.suspicion_score;
    let fps_hurt = avg_fps_delta_percent.is_some_and(|delta| delta <= -3.0);
    let display_path_regressed = suspicion_delta >= 0.15
        || test.display_path_diagnosis.verdict == "likely"
        || test.display_path_diagnosis.verdict == "very_likely";

    match expect {
        Some(DisplayPathExpectation::DirectToOffload) if fps_hurt && display_path_regressed => {
            "uhd630_scanout_likely_hurt_this_run".to_owned()
        }
        Some(DisplayPathExpectation::OffloadToDirect) if !fps_hurt && suspicion_delta <= -0.15 => {
            "direct_scanout_likely_helped_this_run".to_owned()
        }
        _ if fps_hurt && display_path_regressed => "display_path_likely_regressed".to_owned(),
        _ if fps_hurt => "performance_regressed_display_path_unclear".to_owned(),
        _ if suspicion_delta.abs() < 0.10 => "no_clear_display_path_delta".to_owned(),
        _ if suspicion_delta > 0.0 => "display_path_suspicion_increased".to_owned(),
        _ => "display_path_suspicion_decreased".to_owned(),
    }
}

fn comparison_confidence_score(max_severity: u8, evidence_count: usize, strict: bool) -> f64 {
    let base: f64 = match max_severity {
        0 => 0.90,
        1 => 0.65,
        _ => 0.35,
    };
    let evidence_bonus = (evidence_count.min(5) as f64) * 0.02;
    let strict_penalty = if strict && max_severity >= 2 {
        0.10
    } else {
        0.0
    };
    (base + evidence_bonus - strict_penalty).clamp(0.0, 1.0)
}

fn igpu_engine_activity(analysis: &ReportAnalysisJson) -> f64 {
    let summary = &analysis.gpu_engine_activity;
    (summary.igpu_render_activity_near_outliers + summary.igpu_blitter_activity_near_outliers)
        as f64
}

fn same_scanout_gpu(baseline: &ReportAnalysisJson, test: &ReportAnalysisJson) -> bool {
    display_scanout_driver(baseline)
        .zip(display_scanout_driver(test))
        .is_some_and(|(baseline, test)| baseline == test)
        && display_scanout_card(baseline)
            .zip(display_scanout_card(test))
            .is_none_or(|(baseline, test)| baseline == test)
}

fn rough_count_delta(left: usize, right: usize) -> f64 {
    let base = left.max(1) as f64;
    ((right as f64 - left as f64) / base).abs()
}

fn top_process_comm(analysis: &ReportAnalysisJson) -> Option<&str> {
    analysis
        .session
        .tasks
        .first()
        .map(|task| task.process_comm.as_str())
}

fn display_session_type(analysis: &ReportAnalysisJson) -> Option<&str> {
    analysis
        .session
        .core
        .display_path
        .as_ref()
        .and_then(|path| path.session_type.as_deref())
}

fn display_compositor(analysis: &ReportAnalysisJson) -> Option<&str> {
    analysis
        .session
        .core
        .display_path
        .as_ref()
        .and_then(|path| path.compositor.as_deref())
}

fn display_render_driver(analysis: &ReportAnalysisJson) -> Option<&str> {
    analysis
        .session
        .core
        .display_path
        .as_ref()
        .and_then(|path| path.render_driver.as_deref())
}

fn display_scanout_driver(analysis: &ReportAnalysisJson) -> Option<&str> {
    analysis
        .session
        .core
        .display_path
        .as_ref()
        .and_then(|path| path.scanout_driver.as_deref())
}

fn display_scanout_card(analysis: &ReportAnalysisJson) -> Option<&str> {
    analysis
        .session
        .core
        .display_path
        .as_ref()
        .and_then(|path| path.scanout_card.as_deref())
}

fn display_connector(analysis: &ReportAnalysisJson) -> Option<&str> {
    analysis
        .session
        .core
        .display_path
        .as_ref()
        .and_then(|path| path.connector.as_deref())
}

fn selected_connector(topology: &DisplayTopologySnapshot) -> Option<&ConnectorInfo> {
    let guess = topology.guessed_path.as_ref()?;
    let scanout_card = guess.scanout_card.as_deref()?;
    let connector_name = guess.connector.as_deref()?;
    topology
        .connectors
        .iter()
        .find(|connector| connector.card == scanout_card && connector.name == connector_name)
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

fn format_optional_count_delta(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:+.0} samples"))
        .unwrap_or_else(|| "n/a".to_owned())
}

fn format_optional_i64(value: Option<i64>) -> String {
    value
        .map(|value| format!("{value:+}"))
        .unwrap_or_else(|| "n/a".to_owned())
}
