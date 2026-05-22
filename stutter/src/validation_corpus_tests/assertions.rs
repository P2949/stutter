use super::{expectation::*, fixture::*};
use crate::{
    diagnosis::{DiagnosisCandidate, StutterCause},
    recorder::SESSION_SCHEMA_VERSION,
    report::{DataQualityLevel, ReportAnalysisJson},
};

pub(super) fn assert_exact_u64(label: &str, expected: Option<u64>, actual: u64) {
    if let Some(expected) = expected {
        assert_eq!(actual, expected, "{label}");
    }
}

pub(super) fn assert_min_u64(label: &str, expected_min: Option<u64>, actual: u64) {
    if let Some(expected_min) = expected_min {
        assert!(
            actual >= expected_min,
            "{label}: expected at least {expected_min}, got {actual}"
        );
    }
}

pub(super) fn assert_metadata_header(name: &str, spec: &FixtureExpectationFile) {
    assert_eq!(
        spec.name, name,
        "fixture metadata name must match fixture directory name"
    );
    assert_eq!(
        spec.schema_version, SESSION_SCHEMA_VERSION,
        "{name} fixture metadata schema_version must match SESSION_SCHEMA_VERSION"
    );
    assert!(
        !spec.source.trim().is_empty(),
        "{name} fixture metadata source must not be empty"
    );
    assert!(
        !spec.quality_expectation.trim().is_empty(),
        "{name} fixture metadata quality_expectation must not be empty"
    );
    assert!(
        !spec.description.trim().is_empty(),
        "{name} fixture metadata description must not be empty"
    );
}

pub(super) fn assert_quality_reasons_contain(analysis: &ReportAnalysisJson, needles: &[String]) {
    let text = analysis.data_quality.reasons.join("\n")
        + "\n"
        + &analysis.data_quality.validation_warnings.join("\n")
        + "\n"
        + &analysis.data_quality.validation_errors.join("\n");

    for needle in needles {
        assert!(
            text.to_ascii_lowercase()
                .contains(&needle.to_ascii_lowercase()),
            "missing quality warning/reason {needle:?}; got:\n{text}"
        );
    }
}

pub(super) fn assert_data_quality(analysis: &ReportAnalysisJson, spec: &FixtureExpectationFile) {
    let name = spec.name.as_str();
    let expected_quality = parse_data_quality(&spec.expected.data_quality);

    assert_eq!(
        analysis.data_quality.level,
        expected_quality,
        "wrong data quality for {name}: reasons={:?} validation_errors={:?} validation_warnings={:?}",
        analysis.data_quality.reasons,
        analysis.data_quality.validation_errors,
        analysis.data_quality.validation_warnings,
    );

    if expected_quality == DataQualityLevel::High {
        assert!(
            analysis.data_quality.validation_errors.is_empty(),
            "{name} expected no validation errors: {:?}",
            analysis.data_quality.validation_errors
        );
        assert!(
            analysis.data_quality.validation_warnings.is_empty(),
            "{name} expected no validation warnings: {:?}",
            analysis.data_quality.validation_warnings
        );
    } else {
        assert!(
            !spec.expected.quality_reasons_contain.is_empty(),
            "{name} expected {:?} data quality but fixture.toml did not declare expected.quality_reasons_contain",
            expected_quality
        );
        assert_quality_reasons_contain(analysis, &spec.expected.quality_reasons_contain);
    }
}

pub(super) fn assert_diagnosis(analysis: &ReportAnalysisJson, spec: &FixtureExpectationFile) {
    let name = spec.name.as_str();

    match parse_primary_cause(&spec.expected.primary_cause) {
        ExpectedPrimaryCause::Any => {}
        ExpectedPrimaryCause::Cause(expected_cause) => {
            let diagnosis = primary_diagnosis(analysis)
                .unwrap_or_else(|| panic!("{name} expected a primary diagnosis"));
            assert_eq!(diagnosis.cause, expected_cause, "wrong cause for {name}");

            let accepted_confidence = spec
                .expected
                .accepted_confidence
                .iter()
                .map(|value| parse_confidence(value))
                .collect::<Vec<_>>();
            if !accepted_confidence.is_empty() {
                assert!(
                    accepted_confidence.contains(&diagnosis.confidence),
                    "{name} confidence {:?} not in accepted set {:?}",
                    diagnosis.confidence,
                    accepted_confidence,
                );
            }

            let evidence_text = diagnosis.evidence.join("\n");
            for needle in &spec.expected.evidence.contains {
                assert!(
                    evidence_text.contains(needle),
                    "{name} missing evidence substring {:?}; evidence was:\n{}",
                    needle,
                    evidence_text,
                );
            }
        }
        ExpectedPrimaryCause::NoneOrUnknown => {
            assert!(
                primary_diagnosis(analysis).is_none()
                    || matches!(
                        primary_diagnosis(analysis).unwrap().cause,
                        StutterCause::Unknown
                    ),
                "{name} expected no strong diagnosis, got {:?}",
                primary_diagnosis(analysis).map(|diagnosis| &diagnosis.cause),
            );
        }
    }

    if let Some(required_candidate) = spec.expected.required_candidate.as_deref() {
        let cause = parse_stutter_cause(required_candidate);
        let candidate = find_candidate(analysis, cause)
            .unwrap_or_else(|| panic!("{name} missing required diagnosis candidate {cause:?}"));
        let evidence_text = candidate
            .evidence
            .iter()
            .map(|item| item.message.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        for needle in &spec.expected.required_candidate_evidence {
            assert!(
                evidence_text.contains(needle),
                "{name} required candidate {:?} missing evidence substring {:?}; evidence was:\n{}",
                cause,
                needle,
                evidence_text,
            );
        }
    }
}

pub(super) fn assert_expected_artifacts_values(
    name: &str,
    artifacts: ExpectedArtifacts,
    analysis: &ReportAnalysisJson,
) {
    assert_exact_u64(
        &format!("{name} spike_count"),
        artifacts.spikes,
        analysis.artifacts_summary.spike_count,
    );
    assert_min_u64(
        &format!("{name} spike_count"),
        artifacts.spikes_min,
        analysis.artifacts_summary.spike_count,
    );

    assert_exact_u64(
        &format!("{name} interval_record_count"),
        artifacts.intervals,
        analysis.artifacts_summary.interval_record_count,
    );
    assert_min_u64(
        &format!("{name} interval_record_count"),
        artifacts.intervals_min,
        analysis.artifacts_summary.interval_record_count,
    );

    assert_exact_u64(
        &format!("{name} irq_event_count"),
        artifacts.irq_events,
        analysis.artifacts_summary.irq_event_count,
    );
    assert_min_u64(
        &format!("{name} irq_event_count"),
        artifacts.irq_events_min,
        analysis.artifacts_summary.irq_event_count,
    );

    assert_exact_u64(
        &format!("{name} gpu_sample_count"),
        artifacts.gpu_samples,
        analysis.artifacts_summary.gpu_sample_count,
    );
    assert_min_u64(
        &format!("{name} gpu_sample_count"),
        artifacts.gpu_samples_min,
        analysis.artifacts_summary.gpu_sample_count,
    );

    assert_exact_u64(
        &format!("{name} frame_event_count"),
        artifacts.frames,
        analysis.artifacts_summary.frame_event_count,
    );
    assert_min_u64(
        &format!("{name} frame_event_count"),
        artifacts.frames_min,
        analysis.artifacts_summary.frame_event_count,
    );

    assert_exact_u64(
        &format!("{name} block_io_event_count"),
        artifacts.block_io_events,
        analysis.artifacts_summary.block_io_event_count,
    );
    assert_min_u64(
        &format!("{name} block_io_event_count"),
        artifacts.block_io_events_min,
        analysis.artifacts_summary.block_io_event_count,
    );

    assert_exact_u64(
        &format!("{name} foreground_event_count"),
        artifacts.foreground_events,
        analysis.artifacts_summary.foreground_event_count,
    );
    assert_min_u64(
        &format!("{name} foreground_event_count"),
        artifacts.foreground_events_min,
        analysis.artifacts_summary.foreground_event_count,
    );

    assert_exact_u64(
        &format!("{name} kms_flip_event_count"),
        artifacts.kms_flip_events,
        analysis.artifacts_summary.kms_flip_event_count,
    );
    assert_min_u64(
        &format!("{name} kms_flip_event_count"),
        artifacts.kms_flip_events_min,
        analysis.artifacts_summary.kms_flip_event_count,
    );

    assert_exact_u64(
        &format!("{name} drm_fence_event_count"),
        artifacts.drm_fence_events,
        analysis.artifacts_summary.drm_fence_event_count,
    );
    assert_min_u64(
        &format!("{name} drm_fence_event_count"),
        artifacts.drm_fence_events_min,
        analysis.artifacts_summary.drm_fence_event_count,
    );

    assert_exact_u64(
        &format!("{name} wayland_presentation_event_count"),
        artifacts.wayland_presentation_events,
        analysis.artifacts_summary.wayland_presentation_event_count,
    );
    assert_min_u64(
        &format!("{name} wayland_presentation_event_count"),
        artifacts.wayland_presentation_events_min,
        analysis.artifacts_summary.wayland_presentation_event_count,
    );

    assert_exact_u64(
        &format!("{name} dmabuf_event_count"),
        artifacts.dmabuf_events,
        analysis.artifacts_summary.dmabuf_event_count,
    );
    assert_min_u64(
        &format!("{name} dmabuf_event_count"),
        artifacts.dmabuf_events_min,
        analysis.artifacts_summary.dmabuf_event_count,
    );

    assert_exact_u64(
        &format!("{name} gpu_engine_sample_count"),
        artifacts.gpu_engine_samples,
        analysis.artifacts_summary.gpu_engine_sample_count,
    );
    assert_min_u64(
        &format!("{name} gpu_engine_sample_count"),
        artifacts.gpu_engine_samples_min,
        analysis.artifacts_summary.gpu_engine_sample_count,
    );
}

pub(super) fn assert_artifacts(analysis: &ReportAnalysisJson, spec: &FixtureExpectationFile) {
    let artifacts = ExpectedArtifacts {
        spikes: spec.expected.artifacts.spikes,
        spikes_min: spec.expected.artifacts.spikes_min,
        intervals: spec.expected.artifacts.intervals,
        intervals_min: spec.expected.artifacts.intervals_min,
        irq_events: spec.expected.artifacts.irq_events,
        irq_events_min: spec.expected.artifacts.irq_events_min,
        gpu_samples: spec.expected.artifacts.gpu_samples,
        gpu_samples_min: spec.expected.artifacts.gpu_samples_min,
        frames: spec.expected.artifacts.frames,
        frames_min: spec.expected.artifacts.frames_min,
        block_io_events: spec.expected.artifacts.block_io_events,
        block_io_events_min: spec.expected.artifacts.block_io_events_min,
        foreground_events: spec.expected.artifacts.foreground_events,
        foreground_events_min: spec.expected.artifacts.foreground_events_min,
        kms_flip_events: spec.expected.artifacts.kms_flip_events,
        kms_flip_events_min: spec.expected.artifacts.kms_flip_events_min,
        drm_fence_events: spec.expected.artifacts.drm_fence_events,
        drm_fence_events_min: spec.expected.artifacts.drm_fence_events_min,
        wayland_presentation_events: spec.expected.artifacts.wayland_presentation_events,
        wayland_presentation_events_min: spec.expected.artifacts.wayland_presentation_events_min,
        dmabuf_events: spec.expected.artifacts.dmabuf_events,
        dmabuf_events_min: spec.expected.artifacts.dmabuf_events_min,
        gpu_engine_samples: spec.expected.artifacts.gpu_engine_samples,
        gpu_engine_samples_min: spec.expected.artifacts.gpu_engine_samples_min,
    };

    assert_expected_artifacts_values(spec.name.as_str(), artifacts, analysis);
}

pub(super) fn assert_analysis_json_shape(analysis: &ReportAnalysisJson) {
    let value = serde_json::to_value(analysis).expect("ReportAnalysisJson should serialize");
    let object = value
        .as_object()
        .expect("ReportAnalysisJson should serialize as a JSON object");

    for key in [
        "session",
        "cluster_analysis",
        "frame_diagnoses",
        "frame_pacing",
        "pressure_timeline",
        "artifacts_summary",
        "data_quality",
        "focus_summary",
        "foreground_summary",
        "direct_scanout",
        "dmabuf_path",
        "gpu_engine_activity",
        "display_path_diagnosis",
    ] {
        assert!(
            object.contains_key(key),
            "missing analysis-json key {key}; keys={:?}",
            object.keys().collect::<Vec<_>>()
        );
    }

    let data_quality = object["data_quality"]
        .as_object()
        .expect("analysis-json data_quality should serialize as a JSON object");

    for key in [
        "level",
        "reasons",
        "missing_optional_files",
        "validation_errors",
        "validation_warnings",
        "schema_version",
        "expected_schema_version",
        "event_stream_write_errors",
        "spike_events_truncated",
        "drop_counters_nonzero",
    ] {
        assert!(
            data_quality.contains_key(key),
            "missing data_quality key {key}; keys={:?}",
            data_quality.keys().collect::<Vec<_>>()
        );
    }

    let pressure = object["pressure_timeline"]
        .as_object()
        .expect("analysis-json pressure_timeline should serialize as an object");
    for key in [
        "sample_count",
        "windows",
        "peak_windows",
        "pressure_notes",
        "coverage",
    ] {
        assert!(
            pressure.contains_key(key),
            "missing pressure_timeline key {key}; keys={:?}",
            pressure.keys().collect::<Vec<_>>()
        );
    }

    let frame_pacing = object["frame_pacing"]
        .as_object()
        .expect("analysis-json frame_pacing should serialize as an object");
    for key in ["frame_count", "outlier_count", "outliers", "notes"] {
        assert!(
            frame_pacing.contains_key(key),
            "missing frame_pacing key {key}; keys={:?}",
            frame_pacing.keys().collect::<Vec<_>>()
        );
    }
}

pub(super) fn assert_privacy(analysis: &ReportAnalysisJson, spec: &FixtureExpectationFile) {
    let Some(privacy) = spec.privacy.as_ref() else {
        return;
    };

    assert!(
        privacy.titles_redacted
            && privacy.paths_redacted
            && privacy.hostnames_redacted
            && privacy.usernames_redacted,
        "{} fixture metadata must declare all privacy redaction flags true: {:?}",
        spec.name,
        privacy
    );

    if privacy.titles_redacted {
        assert!(
            analysis.foreground_summary.final_title.is_none()
                || analysis.foreground_summary.final_title.as_deref() == Some("redacted"),
            "{} foreground title must be null or redacted, got {:?}",
            spec.name,
            analysis.foreground_summary.final_title
        );
    }
}

pub(super) fn assert_fixture_from_metadata(name: &str) -> ReportAnalysisJson {
    let path = fixture_path(name);
    let spec = load_fixture_toml(&path.join("fixture.toml"));
    let analysis = build_fixture_analysis(name);

    assert_metadata_header(name, &spec);
    assert_data_quality(&analysis, &spec);
    assert_diagnosis(&analysis, &spec);
    assert_artifacts(&analysis, &spec);
    assert_analysis_json_shape(&analysis);
    assert_privacy(&analysis, &spec);

    analysis
}

pub(super) fn find_candidate(
    analysis: &ReportAnalysisJson,
    cause: StutterCause,
) -> Option<&DiagnosisCandidate> {
    analysis
        .cluster_analysis
        .clusters
        .iter()
        .filter_map(|cluster| cluster.diagnosis.as_ref())
        .chain(
            analysis
                .frame_diagnoses
                .iter()
                .map(|frame| &frame.diagnosis),
        )
        .flat_map(|diagnosis| diagnosis.candidates.iter())
        .find(|candidate| candidate.cause == cause)
}

pub(super) fn no_primary_non_unknown_diagnosis(analysis: &ReportAnalysisJson) -> bool {
    analysis
        .cluster_analysis
        .clusters
        .iter()
        .filter_map(|cluster| cluster.diagnosis.as_ref())
        .all(|diagnosis| matches!(diagnosis.cause, StutterCause::Unknown))
}

pub(super) fn assert_primary_anchor_class_in(
    analysis: &ReportAnalysisJson,
    cause: StutterCause,
    allowed_classes: &[crate::process_tree::TaskClass],
) {
    let cluster = analysis
        .cluster_analysis
        .clusters
        .iter()
        .find(|cluster| {
            cluster
                .diagnosis
                .as_ref()
                .is_some_and(|diagnosis| diagnosis.cause == cause)
        })
        .unwrap_or_else(|| panic!("missing primary diagnosis cluster for {cause:?}"));

    let anchor_class = cluster
        .anchor_class
        .unwrap_or_else(|| panic!("missing anchor_class for primary diagnosis {cause:?}"));

    assert!(
        allowed_classes.contains(&anchor_class),
        "primary diagnosis {cause:?} had anchor_class {:?}, expected one of {:?}",
        anchor_class,
        allowed_classes
    );
}

pub(super) fn assert_candidate_contains(
    analysis: &ReportAnalysisJson,
    cause: StutterCause,
    evidence_substrings: &[&str],
) {
    let candidate =
        find_candidate(analysis, cause).unwrap_or_else(|| panic!("missing candidate {cause:?}"));
    let evidence = candidate
        .evidence
        .iter()
        .map(|item| item.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    for needle in evidence_substrings {
        assert!(
            evidence.contains(needle),
            "missing evidence substring {:?}; evidence was:\n{}",
            needle,
            evidence
        );
    }
}
