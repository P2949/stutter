use super::{DisplayPathExpectation, DisplayPathVerdictReason};
use crate::report::ReportAnalysisJson;

pub(super) struct DisplayPathVerdictDecision {
    pub(super) label: String,
    pub(super) reason: DisplayPathVerdictReason,
}

pub(super) fn compare_verdict(
    expect: Option<DisplayPathExpectation>,
    avg_fps_delta_percent: Option<f64>,
    baseline: &ReportAnalysisJson,
    test: &ReportAnalysisJson,
    warnings: &[String],
) -> DisplayPathVerdictDecision {
    let suspicion_delta = test.display_path_diagnosis.suspicion_score
        - baseline.display_path_diagnosis.suspicion_score;
    let facts = VerdictFacts {
        expect,
        avg_fps_delta_percent,
        suspicion_delta,
        test_diagnosis_verdict: test.display_path_diagnosis.verdict.as_str(),
        baseline_cross_gpu: baseline.display_path_diagnosis.is_cross_gpu,
        test_cross_gpu: test.display_path_diagnosis.is_cross_gpu,
        same_scanout_gpu: same_scanout_gpu(baseline, test),
        cross_gpu_fence_detected: cross_gpu_fence_detected(test),
        igpu_engine_active: igpu_engine_active(test),
        topology_mismatch: has_topology_mismatch(warnings),
    };
    DisplayPathVerdictDecision {
        label: verdict_label(&facts).to_owned(),
        reason: verdict_reason(&facts),
    }
}

pub(super) fn comparison_confidence_score(
    max_severity: u8,
    evidence_count: usize,
    strict: bool,
) -> f64 {
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

struct VerdictFacts<'a> {
    expect: Option<DisplayPathExpectation>,
    avg_fps_delta_percent: Option<f64>,
    suspicion_delta: f64,
    test_diagnosis_verdict: &'a str,
    baseline_cross_gpu: Option<bool>,
    test_cross_gpu: Option<bool>,
    same_scanout_gpu: bool,
    cross_gpu_fence_detected: bool,
    igpu_engine_active: bool,
    topology_mismatch: bool,
}

fn verdict_label(facts: &VerdictFacts<'_>) -> &'static str {
    let fps_hurt = facts
        .avg_fps_delta_percent
        .is_some_and(|delta| delta <= -3.0);
    let display_path_regressed = facts.suspicion_delta >= 0.15
        || facts.test_diagnosis_verdict == "likely"
        || facts.test_diagnosis_verdict == "very_likely";

    match facts.expect {
        Some(DisplayPathExpectation::DirectToOffload) if fps_hurt && display_path_regressed => {
            "uhd630_scanout_likely_hurt_this_run"
        }
        Some(DisplayPathExpectation::OffloadToDirect)
            if !fps_hurt && facts.suspicion_delta <= -0.15 =>
        {
            "direct_scanout_likely_helped_this_run"
        }
        _ if fps_hurt && display_path_regressed => "display_path_likely_regressed",
        _ if fps_hurt => "performance_regressed_display_path_unclear",
        _ if facts.suspicion_delta.abs() < 0.10 => "no_clear_display_path_delta",
        _ if facts.suspicion_delta > 0.0 => "display_path_suspicion_increased",
        _ => "display_path_suspicion_decreased",
    }
}

fn verdict_reason(facts: &VerdictFacts<'_>) -> DisplayPathVerdictReason {
    if facts.topology_mismatch {
        return DisplayPathVerdictReason::TopologyMismatch;
    }
    if facts.baseline_cross_gpu.is_none() || facts.test_cross_gpu.is_none() {
        return DisplayPathVerdictReason::MissingEvidence;
    }
    if facts.same_scanout_gpu {
        return DisplayPathVerdictReason::SameScanoutGpu;
    }
    if facts.cross_gpu_fence_detected {
        return DisplayPathVerdictReason::CrossGpuFenceDetected;
    }
    if facts.igpu_engine_active {
        return DisplayPathVerdictReason::IgpuEngineActive;
    }
    DisplayPathVerdictReason::MissingEvidence
}

fn cross_gpu_fence_detected(analysis: &ReportAnalysisJson) -> bool {
    analysis.cross_gpu_fence.candidate_count > 0
        || analysis.cross_gpu_fence.high_confidence_count > 0
        || analysis.drm_fence_timing.cross_gpu_candidate_count > 0
}

fn igpu_engine_active(analysis: &ReportAnalysisJson) -> bool {
    analysis
        .gpu_engine_activity
        .igpu_blitter_activity_near_outliers
        > 0
        || analysis
            .gpu_engine_activity
            .igpu_render_activity_near_outliers
            > 0
}

fn same_scanout_gpu(baseline: &ReportAnalysisJson, test: &ReportAnalysisJson) -> bool {
    scanout_driver(baseline)
        .zip(scanout_driver(test))
        .is_some_and(|(baseline, test)| baseline == test)
        && scanout_card(baseline)
            .zip(scanout_card(test))
            .is_none_or(|(baseline, test)| baseline == test)
}

fn scanout_driver(analysis: &ReportAnalysisJson) -> Option<&str> {
    analysis
        .session
        .core
        .display_path
        .as_ref()
        .and_then(|path| path.scanout_driver.as_deref())
        .or(analysis.display_path_diagnosis.scanout_gpu.as_deref())
}

fn scanout_card(analysis: &ReportAnalysisJson) -> Option<&str> {
    analysis
        .session
        .core
        .display_path
        .as_ref()
        .and_then(|path| path.scanout_card.as_deref())
}

fn has_topology_mismatch(warnings: &[String]) -> bool {
    warnings.iter().any(|warning| {
        warning.contains("display EDID differs")
            || warning.contains("different refresh modes")
            || warning.contains("render GPU changed")
            || warning.contains("connector differs")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> VerdictFacts<'static> {
        VerdictFacts {
            expect: None,
            avg_fps_delta_percent: None,
            suspicion_delta: 0.0,
            test_diagnosis_verdict: "low",
            baseline_cross_gpu: Some(false),
            test_cross_gpu: Some(false),
            same_scanout_gpu: true,
            cross_gpu_fence_detected: false,
            igpu_engine_active: false,
            topology_mismatch: false,
        }
    }

    #[test]
    fn verdict_reason_prefers_topology_mismatch() {
        let mut facts = facts();
        facts.topology_mismatch = true;
        facts.cross_gpu_fence_detected = true;

        assert_eq!(
            verdict_reason(&facts),
            DisplayPathVerdictReason::TopologyMismatch
        );
    }

    #[test]
    fn verdict_reason_tracks_cross_gpu_fence() {
        let mut facts = facts();
        facts.same_scanout_gpu = false;
        facts.test_cross_gpu = Some(true);
        facts.cross_gpu_fence_detected = true;

        assert_eq!(
            verdict_reason(&facts),
            DisplayPathVerdictReason::CrossGpuFenceDetected
        );
    }

    #[test]
    fn verdict_reason_tracks_missing_evidence() {
        let mut facts = facts();
        facts.baseline_cross_gpu = None;

        assert_eq!(
            verdict_reason(&facts),
            DisplayPathVerdictReason::MissingEvidence
        );
    }
}
