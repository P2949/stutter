use std::path::Path;

use crate::{artifacts::ArtifactSelection, session_io};

mod evidence;
mod model;
mod render;
mod validate;
mod verdict;

use evidence::{
    CompareEvidenceDeltas, compare_evidence, count_long_kms_flips, delta, display_path_label, fps,
    igpu_engine_activity, role_fence_p99_ms, scheduler_p99_ms,
};
pub use model::{
    DisplayPathCompareInput, DisplayPathCompareOutput, DisplayPathCostSummary,
    DisplayPathExpectation, DisplayPathVerdictReason,
};
use render::print_display_path_compare;
use validate::{
    validate_comparability, validate_display_path_expectation, validate_probe_match,
    validate_topology_match,
};
use verdict::{compare_verdict, comparison_confidence_score};

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

    let likely_causes = likely_causes(LikelyCauseInput {
        baseline: &baseline,
        test: &test,
        display_side_fence_wait_p99_delta_ms,
        wayland_component_delta_ms,
        igpu_engine_activity_delta,
        dmabuf_copy_required_delta,
    });

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
    let verdict = compare_verdict(expect, avg_fps_delta_percent, &baseline, &test, &warnings);
    let confidence_score = comparison_confidence_score(max_severity, evidence.len(), strict);

    Ok(DisplayPathCompareOutput {
        display_path_cost: DisplayPathCostSummary {
            verdict: verdict.label,
            verdict_reason: verdict.reason,
            confidence_score,
            evidence,
            missing_evidence,
            baseline_label: display_path_label(&baseline),
            test_label: display_path_label(&test),
            comparison_quality: comparison_quality(max_severity, &warnings),
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
            notes: comparison_notes(),
        },
    })
}

struct LikelyCauseInput<'a> {
    baseline: &'a crate::report::ReportAnalysisJson,
    test: &'a crate::report::ReportAnalysisJson,
    display_side_fence_wait_p99_delta_ms: Option<f64>,
    wayland_component_delta_ms: Option<f64>,
    igpu_engine_activity_delta: Option<f64>,
    dmabuf_copy_required_delta: Option<i64>,
}

fn likely_causes(input: LikelyCauseInput<'_>) -> Vec<String> {
    let mut likely_causes = Vec::new();
    if input
        .display_side_fence_wait_p99_delta_ms
        .is_some_and(|value| value > 1.0)
        || (input.test.drm_fence_timing.cross_gpu_candidate_count as i64
            - input.baseline.drm_fence_timing.cross_gpu_candidate_count as i64)
            > 0
    {
        likely_causes.push("cross_gpu_fence_wait_candidate".to_owned());
    }
    if input
        .test
        .wayland_presentation
        .compositor_queue_candidate_count
        .saturating_sub(
            input
                .baseline
                .wayland_presentation
                .compositor_queue_candidate_count,
        )
        > 0
        || input
            .wayland_component_delta_ms
            .is_some_and(|value| value > 1.0)
    {
        likely_causes.push("wayland_presentation_queue_candidate".to_owned());
    }
    if input
        .test
        .kms_timing
        .p99_flip_ms
        .zip(input.baseline.kms_timing.p99_flip_ms)
        .is_some_and(|(test, baseline)| test - baseline > 1.0)
    {
        likely_causes.push("scanout_kms_pageflip_candidate".to_owned());
    }
    if input
        .igpu_engine_activity_delta
        .is_some_and(|value| value > 0.0)
    {
        likely_causes.push("igpu_blitter_activity_near_outliers".to_owned());
    }
    if input
        .dmabuf_copy_required_delta
        .is_some_and(|value| value > 0)
    {
        likely_causes.push("dmabuf_copy_required_candidate".to_owned());
    }
    likely_causes
}

fn comparison_quality(max_severity: u8, warnings: &[String]) -> String {
    match max_severity {
        0 if warnings.is_empty() => "high",
        2.. => "low",
        _ => "medium",
    }
    .to_owned()
}

fn comparison_notes() -> Vec<String> {
    vec![
        "This is an A/B estimate, not direct photon latency.".to_owned(),
        "Input-to-photon latency requires external measurement hardware.".to_owned(),
        "If scheduler latency worsened too, do not attribute the delta to display path alone."
            .to_owned(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_path(name: &str) -> std::path::PathBuf {
        crate::test_fixture_builder::fixture_path(name)
    }

    #[test]
    fn compare_output_records_cross_gpu_fence_reason() {
        let output = compare_display_path_with_options(
            &fixture_path("direct_gpu_clean"),
            &fixture_path("uhd630_cross_gpu_fence_wait"),
            Some(DisplayPathExpectation::DirectToOffload),
            false,
        )
        .expect("display path comparison should succeed");

        assert_eq!(
            output.display_path_cost.verdict_reason,
            DisplayPathVerdictReason::CrossGpuFenceDetected
        );
    }

    #[test]
    fn compare_output_records_igpu_engine_reason() {
        let output = compare_display_path_with_options(
            &fixture_path("direct_gpu_clean"),
            &fixture_path("uhd630_composited_blitter"),
            Some(DisplayPathExpectation::DirectToOffload),
            false,
        )
        .expect("display path comparison should succeed");

        assert_eq!(
            output.display_path_cost.verdict_reason,
            DisplayPathVerdictReason::IgpuEngineActive
        );
    }

    #[test]
    fn compare_output_records_missing_evidence_reason() {
        let output = compare_display_path_with_options(
            &fixture_path("missing_evidence_unknown"),
            &fixture_path("missing_evidence_unknown"),
            Some(DisplayPathExpectation::Unknown),
            false,
        )
        .expect("display path comparison should succeed");

        assert_eq!(
            output.display_path_cost.verdict_reason,
            DisplayPathVerdictReason::MissingEvidence
        );
    }

    #[test]
    fn compare_output_records_same_scanout_reason() {
        let output = compare_display_path_with_options(
            &fixture_path("direct_gpu_clean"),
            &fixture_path("direct_gpu_clean"),
            Some(DisplayPathExpectation::Unknown),
            false,
        )
        .expect("display path comparison should succeed");

        assert_eq!(
            output.display_path_cost.verdict_reason,
            DisplayPathVerdictReason::SameScanoutGpu
        );
    }
}
