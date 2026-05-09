use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::{
    diagnosis::{Confidence, Diagnosis, DiagnosisCandidate, StutterCause},
    recorder::SESSION_SCHEMA_VERSION,
    report::{self, DataQualityLevel, ReportAnalysisJson},
    test_fixture_builder,
};

#[allow(dead_code)]
#[derive(Debug)]
struct ExpectedFixture<'a> {
    name: &'a str,
    expected_primary: Option<StutterCause>,
    accepted_confidence: &'a [Confidence],
    expected_quality: DataQualityLevel,
    evidence_substrings: &'a [&'a str],
    expected_artifacts: ExpectedArtifacts,
    require_json_shape: bool,
}

#[allow(dead_code)]
#[derive(Debug, Default, Clone, Copy)]
struct ExpectedArtifacts {
    spikes: Option<u64>,
    spikes_min: Option<u64>,
    intervals: Option<u64>,
    intervals_min: Option<u64>,
    irq_events: Option<u64>,
    irq_events_min: Option<u64>,
    gpu_samples: Option<u64>,
    gpu_samples_min: Option<u64>,
    frames: Option<u64>,
    frames_min: Option<u64>,
    block_io_events: Option<u64>,
    block_io_events_min: Option<u64>,
    foreground_events: Option<u64>,
    foreground_events_min: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct FixtureExpectationFile {
    name: String,
    schema_version: u32,
    source: String,
    #[serde(default)]
    quality_expectation: String,
    #[serde(default)]
    description: String,
    expected: ExpectedFromToml,
    #[serde(default)]
    privacy: Option<PrivacyExpectations>,
}

#[derive(Debug, Deserialize)]
struct ExpectedFromToml {
    primary_cause: String,
    #[serde(default)]
    required_candidate: Option<String>,
    #[serde(default)]
    required_candidate_evidence: Vec<String>,
    #[serde(default)]
    accepted_confidence: Vec<String>,
    #[serde(default)]
    quality_reasons_contain: Vec<String>,
    data_quality: String,
    #[serde(default)]
    artifacts: ExpectedArtifactsFromToml,
    #[serde(default)]
    evidence: ExpectedEvidenceFromToml,
}

#[derive(Debug, Default, Deserialize)]
struct ExpectedArtifactsFromToml {
    spikes: Option<u64>,
    spikes_min: Option<u64>,
    intervals: Option<u64>,
    intervals_min: Option<u64>,
    irq_events: Option<u64>,
    irq_events_min: Option<u64>,
    gpu_samples: Option<u64>,
    gpu_samples_min: Option<u64>,
    frames: Option<u64>,
    frames_min: Option<u64>,
    block_io_events: Option<u64>,
    block_io_events_min: Option<u64>,
    foreground_events: Option<u64>,
    foreground_events_min: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
struct ExpectedEvidenceFromToml {
    contains: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct PrivacyExpectations {
    #[serde(default)]
    titles_redacted: bool,
    #[serde(default)]
    paths_redacted: bool,
    #[serde(default)]
    hostnames_redacted: bool,
    #[serde(default)]
    usernames_redacted: bool,
}

enum ExpectedPrimaryCause {
    Any,
    NoneOrUnknown,
    Cause(StutterCause),
}

fn fixture_path(name: &str) -> PathBuf {
    test_fixture_builder::fixture_path(name)
}

fn load_fixture_toml(path: &Path) -> FixtureExpectationFile {
    let text = fs::read_to_string(path).unwrap_or_else(|err| {
        panic!(
            "failed to read fixture metadata {}: {err:#}",
            path.display()
        )
    });
    toml::from_str(&text).unwrap_or_else(|err| {
        panic!(
            "failed to parse fixture metadata {}: {err:#}",
            path.display()
        )
    })
}

fn build_fixture_analysis(name: &str) -> ReportAnalysisJson {
    report::build_report_analysis(&fixture_path(name), 10, 5, None)
        .unwrap_or_else(|err| panic!("failed to build analysis for {name}: {err:#}"))
}

fn primary_diagnosis(analysis: &ReportAnalysisJson) -> Option<&Diagnosis> {
    analysis
        .cluster_analysis
        .clusters
        .iter()
        .filter_map(|cluster| cluster.diagnosis.as_ref())
        .next()
}

fn parse_data_quality(value: &str) -> DataQualityLevel {
    match value {
        "High" => DataQualityLevel::High,
        "Medium" => DataQualityLevel::Medium,
        "Low" => DataQualityLevel::Low,
        other => panic!("unknown data quality level in fixture metadata: {other}"),
    }
}

fn parse_confidence(value: &str) -> Confidence {
    match value {
        "Low" => Confidence::Low,
        "Medium" => Confidence::Medium,
        "High" => Confidence::High,
        other => panic!("unknown confidence level in fixture metadata: {other}"),
    }
}

fn parse_stutter_cause(value: &str) -> StutterCause {
    match value {
        "CompositorSchedulerDelay" => StutterCause::CompositorSchedulerDelay,
        "GameThreadSchedulerDelay" => StutterCause::GameThreadSchedulerDelay,
        "IrqDelayCandidate" => StutterCause::IrqDelayCandidate,
        "GpuBoundCandidate" => StutterCause::GpuBoundCandidate,
        "BlockIoCandidate" => StutterCause::BlockIoCandidate,
        "CpuPressureCandidate" => StutterCause::CpuPressureCandidate,
        other => panic!("unknown stutter cause in fixture metadata: {other}"),
    }
}

fn parse_primary_cause(value: &str) -> ExpectedPrimaryCause {
    match value {
        "Any" => ExpectedPrimaryCause::Any,
        "Unknown" => ExpectedPrimaryCause::NoneOrUnknown,
        other => ExpectedPrimaryCause::Cause(parse_stutter_cause(other)),
    }
}

fn assert_exact_u64(label: &str, expected: Option<u64>, actual: u64) {
    if let Some(expected) = expected {
        assert_eq!(actual, expected, "{label}");
    }
}

fn assert_min_u64(label: &str, expected_min: Option<u64>, actual: u64) {
    if let Some(expected_min) = expected_min {
        assert!(
            actual >= expected_min,
            "{label}: expected at least {expected_min}, got {actual}"
        );
    }
}

fn assert_metadata_header(name: &str, spec: &FixtureExpectationFile) {
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

fn assert_quality_reasons_contain(analysis: &ReportAnalysisJson, needles: &[String]) {
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

fn assert_data_quality(analysis: &ReportAnalysisJson, spec: &FixtureExpectationFile) {
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

fn assert_data_quality_hard_coded(
    name: &str,
    analysis: &ReportAnalysisJson,
    expected_quality: DataQualityLevel,
) {
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
    }
}

fn assert_diagnosis(analysis: &ReportAnalysisJson, spec: &FixtureExpectationFile) {
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

fn assert_diagnosis_hard_coded(expected: &ExpectedFixture<'_>, analysis: &ReportAnalysisJson) {
    match expected.expected_primary {
        Some(expected_cause) => {
            let diagnosis = primary_diagnosis(analysis)
                .unwrap_or_else(|| panic!("{} expected a primary diagnosis", expected.name));
            assert_eq!(
                diagnosis.cause, expected_cause,
                "wrong cause for {}",
                expected.name
            );

            if !expected.accepted_confidence.is_empty() {
                assert!(
                    expected.accepted_confidence.contains(&diagnosis.confidence),
                    "{} confidence {:?} not in accepted set {:?}",
                    expected.name,
                    diagnosis.confidence,
                    expected.accepted_confidence,
                );
            }

            let evidence_text = diagnosis.evidence.join("\n");
            for needle in expected.evidence_substrings {
                assert!(
                    evidence_text.contains(needle),
                    "{} missing evidence substring {:?}; evidence was:\n{}",
                    expected.name,
                    needle,
                    evidence_text,
                );
            }
        }
        None => {
            assert!(
                primary_diagnosis(analysis).is_none()
                    || matches!(
                        primary_diagnosis(analysis).unwrap().cause,
                        StutterCause::Unknown
                    ),
                "{} expected no strong diagnosis, got {:?}",
                expected.name,
                primary_diagnosis(analysis).map(|diagnosis| &diagnosis.cause),
            );
        }
    }
}

fn assert_expected_artifacts_values(
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
}

fn assert_artifacts(analysis: &ReportAnalysisJson, spec: &FixtureExpectationFile) {
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
    };

    assert_expected_artifacts_values(spec.name.as_str(), artifacts, analysis);
}

fn assert_analysis_json_shape(analysis: &ReportAnalysisJson) {
    let value = serde_json::to_value(analysis).expect("ReportAnalysisJson should serialize");
    let object = value
        .as_object()
        .expect("ReportAnalysisJson should serialize as a JSON object");

    for key in [
        "session",
        "cluster_analysis",
        "frame_diagnoses",
        "pressure_timeline",
        "artifacts_summary",
        "data_quality",
        "focus_summary",
        "foreground_summary",
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
}

fn assert_privacy(analysis: &ReportAnalysisJson, spec: &FixtureExpectationFile) {
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

fn assert_fixture_from_metadata(name: &str) -> ReportAnalysisJson {
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

#[allow(dead_code)]
fn assert_fixture_hard_coded(expected: ExpectedFixture<'_>) -> ReportAnalysisJson {
    let analysis = build_fixture_analysis(expected.name);

    assert_data_quality_hard_coded(expected.name, &analysis, expected.expected_quality);
    assert_diagnosis_hard_coded(&expected, &analysis);
    assert_expected_artifacts_values(expected.name, expected.expected_artifacts, &analysis);

    if expected.require_json_shape {
        assert_analysis_json_shape(&analysis);
    }

    analysis
}

fn find_candidate(
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

fn no_primary_non_unknown_diagnosis(analysis: &ReportAnalysisJson) -> bool {
    analysis
        .cluster_analysis
        .clusters
        .iter()
        .filter_map(|cluster| cluster.diagnosis.as_ref())
        .all(|diagnosis| matches!(diagnosis.cause, StutterCause::Unknown))
}

fn assert_primary_anchor_class_in(
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

fn assert_candidate_contains(
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

#[test]
fn validation_corpus_cpu_pressure() {
    assert_fixture_from_metadata("cpu_pressure");
}

#[test]
fn pressure_timeline_marks_near_spike_windows() {
    let analysis = build_fixture_analysis("cpu_pressure");
    assert!(analysis.pressure_timeline.sample_count > 0);
    assert!(
        analysis
            .pressure_timeline
            .windows
            .iter()
            .any(|window| window.near_spike),
        "pressure timeline should mark at least one window near a spike"
    );
}

#[test]
fn pressure_timeline_reports_coverage() {
    let analysis = build_fixture_analysis("cpu_pressure");
    assert!(analysis.pressure_timeline.coverage.interval_records_loaded > 0);
    assert!(analysis.pressure_timeline.coverage.has_cpu_psi);
}

#[test]
fn validation_corpus_block_io_stall() {
    assert_fixture_from_metadata("block_io_stall");
}

#[test]
fn validation_corpus_irq_heavy() {
    assert_fixture_from_metadata("irq_heavy");
}

#[test]
fn validation_corpus_gpu_bound_clean_cpu_has_gpu_candidate() {
    let analysis = assert_fixture_from_metadata("gpu_bound_clean_cpu");

    assert_candidate_contains(&analysis, StutterCause::GpuBoundCandidate, &["GPU busy"]);
}

#[test]
fn validation_corpus_real_clean_baseline() {
    let analysis = assert_fixture_from_metadata("real_clean_baseline");

    assert!(analysis.data_quality.validation_errors.is_empty());
    assert!(analysis.data_quality.validation_warnings.is_empty());
    assert!(
        analysis.cluster_analysis.clusters.is_empty()
            || no_primary_non_unknown_diagnosis(&analysis),
        "real_clean_baseline must not produce a non-Unknown primary diagnosis: {:?}",
        analysis
            .cluster_analysis
            .clusters
            .iter()
            .filter_map(|cluster| cluster.diagnosis.as_ref())
            .map(|diagnosis| &diagnosis.cause)
            .collect::<Vec<_>>()
    );
}

#[test]
fn validation_corpus_real_game_thread_scheduler_delay() {
    let analysis = assert_fixture_from_metadata("real_game_thread_scheduler_delay");

    assert_primary_anchor_class_in(
        &analysis,
        StutterCause::GameThreadSchedulerDelay,
        &[
            crate::process_tree::TaskClass::Game,
            crate::process_tree::TaskClass::GameRenderThread,
            crate::process_tree::TaskClass::GameWorkerThread,
            crate::process_tree::TaskClass::GameHelper,
            crate::process_tree::TaskClass::WineServer,
        ],
    );

    assert!(
        analysis.artifacts_summary.spike_count >= 3,
        "real_game_thread_scheduler_delay should contain clustered game/render/main-thread spikes"
    );
    assert!(
        analysis.artifacts_summary.frame_event_count >= 1,
        "real_game_thread_scheduler_delay should contain frame-correlation data near the scheduler spike"
    );
    assert!(
        analysis.artifacts_summary.interval_record_count >= 1,
        "real_game_thread_scheduler_delay should contain interval data so CPU pressure can be ruled out"
    );
    assert_eq!(
        analysis.artifacts_summary.irq_event_count, 0,
        "IRQ evidence should not dominate real_game_thread_scheduler_delay"
    );
    assert_eq!(
        analysis.artifacts_summary.block_io_event_count, 0,
        "block I/O evidence should not dominate real_game_thread_scheduler_delay"
    );
}

#[test]
fn validation_corpus_real_compositor_scheduler_delay() {
    let analysis = assert_fixture_from_metadata("real_compositor_scheduler_delay");

    let diagnosis = primary_diagnosis(&analysis)
        .expect("real_compositor_scheduler_delay expected a primary diagnosis");
    let evidence_text = diagnosis.evidence.join("\n").to_ascii_lowercase();
    assert!(
        evidence_text.contains("compositor thread") || evidence_text.contains("gamescope"),
        "real_compositor_scheduler_delay missing compositor/gamescope evidence; evidence was:\n{}",
        diagnosis.evidence.join("\n")
    );

    assert_primary_anchor_class_in(
        &analysis,
        StutterCause::CompositorSchedulerDelay,
        &[
            crate::process_tree::TaskClass::Compositor,
            crate::process_tree::TaskClass::GameScope,
        ],
    );

    assert!(
        analysis.artifacts_summary.frame_event_count >= 1,
        "real_compositor_scheduler_delay must contain frame data near the scheduler spike"
    );
    assert_eq!(
        analysis.artifacts_summary.irq_event_count, 0,
        "IRQ evidence should not dominate real_compositor_scheduler_delay"
    );
    assert_eq!(
        analysis.artifacts_summary.block_io_event_count, 0,
        "block I/O evidence should not dominate real_compositor_scheduler_delay"
    );
}

#[test]
fn validation_corpus_real_irq_overlap() {
    let analysis = assert_fixture_from_metadata("real_irq_overlap");

    assert!(
        matches!(
            analysis.data_quality.level,
            DataQualityLevel::High | DataQualityLevel::Medium
        ),
        "real_irq_overlap data quality should be High or Medium, got {:?}",
        analysis.data_quality.level
    );
    assert!(analysis.data_quality.validation_errors.is_empty());
    assert!(
        analysis.artifacts_summary.irq_event_count > 0,
        "real_irq_overlap must contain IRQ artifacts"
    );
    assert!(
        analysis.artifacts_summary.irq_event_count >= 4,
        "real_irq_overlap should include multiple IRQ events, including unrelated noise outside the spike window"
    );
    assert_eq!(
        analysis.artifacts_summary.block_io_event_count, 0,
        "block I/O evidence should not dominate real_irq_overlap"
    );
    assert_eq!(
        analysis.artifacts_summary.gpu_sample_count, 0,
        "GPU evidence should not dominate real_irq_overlap"
    );

    let candidate = find_candidate(&analysis, StutterCause::IrqDelayCandidate)
        .expect("real_irq_overlap missing IRQ diagnosis candidate");
    let candidate_evidence = candidate
        .evidence
        .iter()
        .map(|item| item.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        candidate_evidence.contains("IRQ"),
        "real_irq_overlap IRQ candidate evidence did not mention IRQ; evidence was:\n{}",
        candidate_evidence
    );
    assert!(
        !candidate_evidence.contains("147") && !candidate_evidence.contains("148"),
        "real_irq_overlap IRQ candidate evidence should stay focused on the correlated IRQ window and not report unrelated IRQ 147/148 noise; evidence was:\n{}",
        candidate_evidence
    );
}

#[test]
fn validation_corpus_real_gpu_bound_looking() {
    let analysis = assert_fixture_from_metadata("real_gpu_bound_looking");

    assert!(
        analysis.artifacts_summary.gpu_sample_count > 0,
        "real_gpu_bound_looking must contain GPU samples"
    );
    assert!(
        analysis.artifacts_summary.frame_event_count > 0,
        "real_gpu_bound_looking must contain frame events"
    );

    assert_candidate_contains(&analysis, StutterCause::GpuBoundCandidate, &["GPU busy"]);
}

#[test]
fn validation_corpus_real_block_io_overlap() {
    let analysis = assert_fixture_from_metadata("real_block_io_overlap");

    assert!(
        matches!(
            analysis.data_quality.level,
            DataQualityLevel::High | DataQualityLevel::Medium
        ),
        "real_block_io_overlap data quality should be High or Medium, got {:?}",
        analysis.data_quality.level
    );
    assert!(analysis.data_quality.validation_errors.is_empty());
    assert!(
        !analysis
            .data_quality
            .block_io_correlation_basis
            .trim()
            .is_empty(),
        "real_block_io_overlap must report block_io_correlation_basis"
    );
    assert_eq!(
        analysis.data_quality.block_io_correlation_basis, "request-pointer",
        "real_block_io_overlap should use strong request-pointer block I/O correlation"
    );
    assert!(
        analysis.artifacts_summary.block_io_event_count > 0,
        "real_block_io_overlap must contain block I/O artifacts"
    );
    assert!(
        analysis.artifacts_summary.block_io_event_count >= 2,
        "real_block_io_overlap should include one correlated block I/O event and one unrelated event outside the spike window"
    );
    assert_eq!(
        analysis.artifacts_summary.irq_event_count, 0,
        "IRQ evidence should not dominate real_block_io_overlap"
    );
    assert_eq!(
        analysis.artifacts_summary.gpu_sample_count, 0,
        "GPU evidence should not dominate real_block_io_overlap"
    );

    let candidate = find_candidate(&analysis, StutterCause::BlockIoCandidate)
        .expect("real_block_io_overlap missing block I/O diagnosis candidate");
    let candidate_evidence = candidate
        .evidence
        .iter()
        .map(|item| item.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        candidate_evidence.contains("block I/O"),
        "real_block_io_overlap block I/O candidate evidence did not mention block I/O; evidence was:\n{}",
        candidate_evidence
    );
    assert!(
        !candidate_evidence.contains("4,194,304")
            && !candidate_evidence.contains("4194304")
            && !candidate_evidence.contains("43ms"),
        "real_block_io_overlap block I/O evidence should stay focused on the correlated spike-window event and not report unrelated early I/O noise; evidence was:\n{}",
        candidate_evidence
    );
}

#[test]
fn validation_corpus_real_truncated_low_quality() {
    let analysis = assert_fixture_from_metadata("real_truncated_low_quality");

    assert!(
        matches!(
            analysis.data_quality.level,
            DataQualityLevel::Medium | DataQualityLevel::Low
        ),
        "real_truncated_low_quality data quality should be Medium or Low, got {:?}",
        analysis.data_quality.level
    );

    let has_low_quality_signal = analysis.data_quality.spike_events_truncated
        || analysis.data_quality.event_stream_write_errors > 0
        || analysis.data_quality.drop_counters_nonzero;
    assert!(
        has_low_quality_signal,
        "real_truncated_low_quality must expose truncation, event stream write errors, or nonzero drop counters"
    );

    let quality_text = analysis
        .data_quality
        .reasons
        .iter()
        .chain(analysis.data_quality.validation_warnings.iter())
        .chain(analysis.data_quality.validation_errors.iter())
        .map(|message| message.as_str())
        .collect::<Vec<_>>()
        .join("\n")
        .to_ascii_lowercase();

    assert!(
        quality_text.contains("truncated")
            || quality_text.contains("drop")
            || quality_text.contains("write error"),
        "real_truncated_low_quality quality reasons/warnings/errors must mention truncated, drop, or write error; text was:\n{}",
        quality_text
    );

    assert!(
        primary_diagnosis(&analysis).is_none() || no_primary_non_unknown_diagnosis(&analysis),
        "real_truncated_low_quality must not assert a strong non-Unknown diagnosis: {:?}",
        analysis
            .cluster_analysis
            .clusters
            .iter()
            .filter_map(|cluster| cluster.diagnosis.as_ref())
            .map(|diagnosis| &diagnosis.cause)
            .collect::<Vec<_>>()
    );

    assert!(
        analysis.data_quality.spike_events_truncated,
        "real_truncated_low_quality should exercise spike_events_truncated"
    );
    assert!(
        analysis.data_quality.spike_events_dropped_count > 0,
        "real_truncated_low_quality should exercise spike_events_dropped_count"
    );
    assert!(
        analysis.data_quality.drop_counters_nonzero,
        "real_truncated_low_quality should exercise drop_counters_nonzero"
    );
}

#[test]
fn validation_corpus_real_foreground_window() {
    let analysis = assert_fixture_from_metadata("real_foreground_window");

    assert!(
        analysis.foreground_summary.enabled,
        "real_foreground_window should report foreground tracking as enabled"
    );
    assert!(
        analysis.foreground_summary.event_count > 0,
        "real_foreground_window should contain at least one foreground event"
    );
    assert!(
        analysis.foreground_summary.final_pid.is_some()
            || analysis.foreground_summary.final_app_id.is_some()
            || analysis.foreground_summary.final_class.is_some(),
        "real_foreground_window should preserve final foreground pid, app_id, or class"
    );
    assert!(
        analysis.foreground_summary.final_title.is_none()
            || analysis.foreground_summary.final_title.as_deref() == Some("redacted"),
        "real_foreground_window title must be null or redacted, got {:?}",
        analysis.foreground_summary.final_title
    );
    assert_eq!(analysis.foreground_summary.final_pid, Some(5701));
    assert_eq!(
        analysis.foreground_summary.final_app_id.as_deref(),
        Some("steam_app_sanitized")
    );
    assert_eq!(
        analysis.foreground_summary.final_class.as_deref(),
        Some("steam_app_sanitized")
    );
    assert!(
        analysis.artifacts_summary.foreground_event_count > 0,
        "real_foreground_window must contain foreground_events.json artifacts"
    );
    assert!(
        analysis.artifacts_summary.spike_count >= 3,
        "real_foreground_window must contain a scheduler cluster near the foreground event"
    );

    let annotated_cluster = analysis.cluster_analysis.clusters.iter().find(|cluster| {
        cluster.foreground_pid == Some(5701)
            || cluster.foreground_app_id.as_deref() == Some("steam_app_sanitized")
            || cluster.foreground_class.as_deref() == Some("steam_app_sanitized")
    });
    assert!(
        annotated_cluster.is_some(),
        "real_foreground_window expected a cluster annotated with foreground pid/app/class; clusters={:?}",
        analysis.cluster_analysis.clusters
    );

    let cluster = annotated_cluster.expect("checked above");
    assert_eq!(cluster.foreground_pid, Some(5701));
    assert_eq!(
        cluster.foreground_app_id.as_deref(),
        Some("steam_app_sanitized")
    );
    assert_eq!(
        cluster.foreground_class.as_deref(),
        Some("steam_app_sanitized")
    );
    assert!(
        cluster.foreground_confidence.is_some(),
        "real_foreground_window annotated cluster should carry foreground confidence"
    );
}

#[test]
fn validation_corpus_real_community_rules_classification() {
    let analysis = assert_fixture_from_metadata("real_community_rules_classification");

    let classified_task = analysis
        .session
        .tasks
        .iter()
        .find(|task| task.comm == "community-game")
        .expect("missing community-rule-classified game task");

    assert_eq!(
        classified_task.class,
        crate::process_tree::TaskClass::Game,
        "report fixture should contain final class Game for the community-rule-classified task"
    );
    assert_eq!(classified_task.process_comm.as_ref(), "community-game");

    assert_primary_anchor_class_in(
        &analysis,
        StutterCause::GameThreadSchedulerDelay,
        &[
            crate::process_tree::TaskClass::Game,
            crate::process_tree::TaskClass::GameRenderThread,
            crate::process_tree::TaskClass::GameWorkerThread,
            crate::process_tree::TaskClass::GameHelper,
            crate::process_tree::TaskClass::WineServer,
        ],
    );

    assert!(
        analysis.artifacts_summary.spike_count >= 3,
        "real_community_rules_classification should contain clustered game-relevant spikes"
    );
    assert!(
        analysis.artifacts_summary.frame_event_count > 0,
        "real_community_rules_classification should include frame context for downstream diagnosis"
    );
}

#[test]
fn validation_corpus_game_thread_scheduler_delay() {
    assert_fixture_from_metadata("game_thread_scheduler_delay");
}

#[test]
fn validation_corpus_compositor_scheduler_delay() {
    assert_fixture_from_metadata("compositor_scheduler_delay");
}

#[test]
fn validation_corpus_foreground_window() {
    let analysis = assert_fixture_from_metadata("foreground_window");

    assert_eq!(analysis.foreground_summary.final_pid, Some(5701));
    assert_eq!(
        analysis.foreground_summary.final_app_id.as_deref(),
        Some("steam_app_sanitized")
    );
    assert_eq!(
        analysis.foreground_summary.final_class.as_deref(),
        Some("steam_app_sanitized")
    );
    assert!(
        analysis.foreground_summary.final_title.is_none(),
        "foreground title must stay redacted"
    );
}

#[test]
fn validation_corpus_community_rules_classification() {
    let analysis = assert_fixture_from_metadata("community_rules_classification");

    let task = analysis
        .session
        .tasks
        .iter()
        .find(|task| task.comm == "community-game")
        .expect("missing community-classified task");

    assert_eq!(task.class, crate::process_tree::TaskClass::Game);
    assert_eq!(task.process_comm.as_ref(), "community-game");
}

#[test]
fn validation_corpus_clean_run_is_high_quality_without_false_diagnosis() {
    let analysis = assert_fixture_from_metadata("clean_run");

    assert!(analysis.data_quality.validation_errors.is_empty());
    assert!(analysis.cluster_analysis.clusters.is_empty());
}

#[test]
fn validation_corpus_truncated_drop_counters_is_not_high_quality() {
    let analysis = assert_fixture_from_metadata("truncated_drop_counters");

    assert!(analysis.data_quality.spike_events_truncated);
    assert!(analysis.data_quality.drop_counters_nonzero);
    assert!(analysis.data_quality.spike_events_dropped_count > 0);
    assert_eq!(
        analysis.data_quality.spike_events_retained_count,
        analysis.artifacts_summary.spike_count
    );
    assert_quality_reasons_contain(&analysis, &["truncated".to_owned(), "drop".to_owned()]);
}

#[test]
fn validation_corpus_reused_tid_no_contamination() {
    let analysis = assert_fixture_from_metadata("reused_tid_no_contamination");

    let reused_tasks = analysis
        .session
        .tasks
        .iter()
        .filter(|task| task.task == 4242)
        .collect::<Vec<_>>();
    assert_eq!(reused_tasks.len(), 2, "reused TID should remain split");

    let old_task = reused_tasks
        .iter()
        .find(|task| task.comm == "old-worker")
        .copied()
        .expect("missing old logical task");
    let new_task = reused_tasks
        .iter()
        .find(|task| task.comm == "new-worker")
        .copied()
        .expect("missing new logical task");

    assert_eq!(old_task.latency.samples, 2);
    assert_eq!(new_task.latency.samples, 3);
    assert_ne!(old_task.process_pid, new_task.process_pid);
    assert_ne!(
        old_task.process_starttime_ticks,
        new_task.process_starttime_ticks
    );
    assert_ne!(old_task.task_starttime_ticks, new_task.task_starttime_ticks);
    assert_ne!(old_task.exe_ino, new_task.exe_ino);
    assert!(
        !reused_tasks
            .iter()
            .any(|task| task.latency.samples == 5 || task.latency.max_ns > 1_200_000),
        "reused TID stats appear to be combined: {reused_tasks:?}"
    );
}

#[test]
fn validation_corpus_old_schema_warns_without_rejecting() {
    let analysis = assert_fixture_from_metadata("old_schema_warning");

    assert_eq!(
        analysis.data_quality.schema_version,
        SESSION_SCHEMA_VERSION - 1
    );
    assert_eq!(
        analysis.data_quality.expected_schema_version,
        SESSION_SCHEMA_VERSION
    );
    assert_quality_reasons_contain(&analysis, &["older than current".to_owned()]);
    assert!(
        analysis.data_quality.validation_errors.is_empty(),
        "old schema should warn, not error: {:?}",
        analysis.data_quality.validation_errors
    );
}

#[test]
#[ignore]
fn regenerate_public_examples_v21() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .expect("stutter crate manifest should have a workspace parent");
    let root = workspace_root
        .join("docs")
        .join("examples")
        .join("artifacts")
        .join("v21");

    test_fixture_builder::write_public_examples_v21(&root)
        .unwrap_or_else(|err| panic!("failed to regenerate public v21 examples: {err:#}"));
}

#[test]
#[ignore]
fn regenerate_validation_corpus() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("runs");
    test_fixture_builder::write_validation_corpus(&root)
        .unwrap_or_else(|err| panic!("failed to regenerate validation corpus: {err:#}"));
}
