use std::{fs, path::PathBuf};

use serde::Deserialize;

use crate::{
    diagnosis::{Confidence, Diagnosis, DiagnosisCandidate, StutterCause},
    recorder::SESSION_SCHEMA_VERSION,
    report::{self, DataQualityLevel, ReportAnalysisJson},
    test_fixture_builder,
};

#[derive(Debug, Deserialize)]
struct FixtureToml {
    name: String,
    schema_version: u32,
    source: String,
    quality_expectation: String,
    description: String,
    expected: FixtureTomlExpected,
    privacy: FixtureTomlPrivacy,
}

#[derive(Debug, Deserialize)]
struct FixtureTomlExpected {
    primary_cause: String,
    accepted_confidence: Vec<String>,
    data_quality: String,
    artifacts: FixtureTomlArtifacts,
    evidence: FixtureTomlEvidence,
}

#[derive(Debug, Default, Deserialize)]
struct FixtureTomlArtifacts {
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
struct FixtureTomlEvidence {
    contains: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct FixtureTomlPrivacy {
    titles_redacted: bool,
    paths_redacted: bool,
    hostnames_redacted: bool,
    usernames_redacted: bool,
}

fn fixture_path(name: &str) -> PathBuf {
    test_fixture_builder::fixture_path(name)
}

fn fixture_toml_path(name: &str) -> PathBuf {
    fixture_path(name).join("fixture.toml")
}

fn load_fixture_toml(name: &str) -> FixtureToml {
    let path = fixture_toml_path(name);
    let text = fs::read_to_string(&path).unwrap_or_else(|err| {
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

fn parse_primary_cause(value: &str) -> Option<StutterCause> {
    match value {
        "CompositorSchedulerDelay" => Some(StutterCause::CompositorSchedulerDelay),
        "GameThreadSchedulerDelay" => Some(StutterCause::GameThreadSchedulerDelay),
        "IrqDelayCandidate" => Some(StutterCause::IrqDelayCandidate),
        "GpuBoundCandidate" => Some(StutterCause::GpuBoundCandidate),
        "BlockIoCandidate" => Some(StutterCause::BlockIoCandidate),
        "CpuPressureCandidate" => Some(StutterCause::CpuPressureCandidate),
        "Unknown" => None,
        other => panic!("unknown stutter cause in fixture metadata: {other}"),
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

fn assert_artifacts_from_metadata(
    name: &str,
    analysis: &ReportAnalysisJson,
    metadata: &FixtureToml,
) {
    let artifacts = &metadata.expected.artifacts;

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

fn assert_fixture_from_metadata(name: &str) -> (ReportAnalysisJson, FixtureToml) {
    let metadata = load_fixture_toml(name);
    let analysis = build_fixture_analysis(name);

    assert_eq!(
        metadata.name, name,
        "fixture metadata name must match fixture directory name"
    );
    assert_eq!(
        metadata.schema_version, SESSION_SCHEMA_VERSION,
        "{name} fixture metadata schema_version must match SESSION_SCHEMA_VERSION"
    );
    assert!(
        !metadata.source.trim().is_empty(),
        "{name} fixture metadata source must not be empty"
    );
    assert!(
        !metadata.quality_expectation.trim().is_empty(),
        "{name} fixture metadata quality_expectation must not be empty"
    );
    assert!(
        !metadata.description.trim().is_empty(),
        "{name} fixture metadata description must not be empty"
    );
    assert!(
        metadata.privacy.titles_redacted
            && metadata.privacy.paths_redacted
            && metadata.privacy.hostnames_redacted
            && metadata.privacy.usernames_redacted,
        "{name} fixture metadata must declare all privacy redaction flags true: {:?}",
        metadata.privacy
    );

    let expected_quality = parse_data_quality(&metadata.expected.data_quality);
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

    if let Some(expected_cause) = parse_primary_cause(&metadata.expected.primary_cause) {
        let diagnosis = primary_diagnosis(&analysis)
            .unwrap_or_else(|| panic!("{name} expected a primary diagnosis"));
        assert_eq!(diagnosis.cause, expected_cause, "wrong cause for {name}");

        let accepted_confidence = metadata
            .expected
            .accepted_confidence
            .iter()
            .map(|value| parse_confidence(value))
            .collect::<Vec<_>>();
        assert!(
            accepted_confidence.contains(&diagnosis.confidence),
            "{name} confidence {:?} not in accepted set {:?}",
            diagnosis.confidence,
            accepted_confidence,
        );

        let evidence_text = diagnosis.evidence.join("\n");
        for needle in &metadata.expected.evidence.contains {
            assert!(
                evidence_text.contains(needle),
                "{name} missing evidence substring {:?}; evidence was:\n{}",
                needle,
                evidence_text,
            );
        }
    } else {
        assert!(
            primary_diagnosis(&analysis).is_none()
                || matches!(
                    primary_diagnosis(&analysis).unwrap().cause,
                    StutterCause::Unknown
                ),
            "{name} expected no strong diagnosis, got {:?}",
            primary_diagnosis(&analysis).map(|diagnosis| &diagnosis.cause),
        );
    }

    assert_artifacts_from_metadata(name, &analysis, &metadata);

    (analysis, metadata)
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
fn validation_corpus_block_io_stall() {
    assert_fixture_from_metadata("block_io_stall");
}

#[test]
fn validation_corpus_irq_heavy() {
    assert_fixture_from_metadata("irq_heavy");
}

#[test]
fn validation_corpus_gpu_bound_clean_cpu_has_gpu_candidate() {
    let (analysis, _) = assert_fixture_from_metadata("gpu_bound_clean_cpu");

    assert_candidate_contains(&analysis, StutterCause::GpuBoundCandidate, &["GPU busy"]);
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
    let (analysis, _) = assert_fixture_from_metadata("foreground_window");

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
    let (analysis, _) = assert_fixture_from_metadata("community_rules_classification");

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
    let (analysis, _) = assert_fixture_from_metadata("clean_run");

    assert!(analysis.data_quality.validation_errors.is_empty());
    assert!(analysis.cluster_analysis.clusters.is_empty());
}

#[test]
fn validation_corpus_truncated_drop_counters_is_not_high_quality() {
    let (analysis, _) = assert_fixture_from_metadata("truncated_drop_counters");

    assert!(analysis.data_quality.spike_events_truncated);
    assert!(analysis.data_quality.drop_counters_nonzero);
    assert!(analysis.data_quality.spike_events_dropped_count > 0);
    assert_eq!(
        analysis.data_quality.spike_events_retained_count,
        analysis.artifacts_summary.spike_count
    );
    let reasons = analysis.data_quality.reasons.join("\n");
    assert!(
        reasons.contains("truncated") || reasons.contains("drop"),
        "expected truncation/drop reason, got: {reasons}"
    );
}

#[test]
fn validation_corpus_reused_tid_no_contamination() {
    let (analysis, _) = assert_fixture_from_metadata("reused_tid_no_contamination");

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
    let (analysis, _) = assert_fixture_from_metadata("old_schema_warning");

    assert_eq!(
        analysis.data_quality.schema_version,
        SESSION_SCHEMA_VERSION - 1
    );
    assert_eq!(
        analysis.data_quality.expected_schema_version,
        SESSION_SCHEMA_VERSION
    );
    assert!(
        analysis
            .data_quality
            .validation_warnings
            .iter()
            .any(|warning| warning.contains("older than current")),
        "warnings: {:?}",
        analysis.data_quality.validation_warnings
    );
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
