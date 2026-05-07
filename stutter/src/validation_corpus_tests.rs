use std::path::PathBuf;

use crate::{
    diagnosis::{Confidence, Diagnosis, DiagnosisCandidate, StutterCause},
    recorder::SESSION_SCHEMA_VERSION,
    report::{self, DataQualityLevel, ReportAnalysisJson},
    test_fixture_builder,
};

struct ExpectedFixture {
    name: &'static str,
    expected_primary: Option<StutterCause>,
    accepted_confidence: &'static [Confidence],
    expected_quality: DataQualityLevel,
    evidence_substrings: &'static [&'static str],
    expected_artifacts: ExpectedArtifacts,
}

#[derive(Default)]
struct ExpectedArtifacts {
    spikes: Option<u64>,
    intervals: Option<u64>,
    irq_events: Option<u64>,
    gpu_samples: Option<u64>,
    frames: Option<u64>,
    block_io_events: Option<u64>,
}

fn fixture_path(name: &str) -> PathBuf {
    test_fixture_builder::fixture_path(name)
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

fn assert_fixture(expected: ExpectedFixture) -> ReportAnalysisJson {
    let analysis = build_fixture_analysis(expected.name);

    assert_eq!(
        analysis.data_quality.level,
        expected.expected_quality,
        "wrong data quality for {}: reasons={:?} validation_errors={:?} validation_warnings={:?}",
        expected.name,
        analysis.data_quality.reasons,
        analysis.data_quality.validation_errors,
        analysis.data_quality.validation_warnings,
    );

    if expected.expected_quality == DataQualityLevel::High {
        assert!(
            analysis.data_quality.validation_errors.is_empty(),
            "{} expected no validation errors: {:?}",
            expected.name,
            analysis.data_quality.validation_errors
        );
        assert!(
            analysis.data_quality.validation_warnings.is_empty(),
            "{} expected no validation warnings: {:?}",
            expected.name,
            analysis.data_quality.validation_warnings
        );
    }

    if let Some(expected_cause) = expected.expected_primary {
        let diagnosis = primary_diagnosis(&analysis)
            .unwrap_or_else(|| panic!("{} expected a primary diagnosis", expected.name));
        assert_eq!(
            diagnosis.cause, expected_cause,
            "wrong cause for {}",
            expected.name
        );
        assert!(
            expected.accepted_confidence.contains(&diagnosis.confidence),
            "{} confidence {:?} not in accepted set {:?}",
            expected.name,
            diagnosis.confidence,
            expected.accepted_confidence,
        );

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
    } else {
        assert!(
            primary_diagnosis(&analysis).is_none()
                || matches!(
                    primary_diagnosis(&analysis).unwrap().cause,
                    StutterCause::Unknown
                ),
            "{} expected no strong diagnosis, got {:?}",
            expected.name,
            primary_diagnosis(&analysis).map(|diagnosis| &diagnosis.cause),
        );
    }

    if let Some(n) = expected.expected_artifacts.spikes {
        assert_eq!(
            analysis.artifacts_summary.spike_count, n,
            "{} spike_count",
            expected.name
        );
    }
    if let Some(n) = expected.expected_artifacts.intervals {
        assert_eq!(
            analysis.artifacts_summary.interval_record_count, n,
            "{} interval_record_count",
            expected.name
        );
    }
    if let Some(n) = expected.expected_artifacts.irq_events {
        assert_eq!(
            analysis.artifacts_summary.irq_event_count, n,
            "{} irq_event_count",
            expected.name
        );
    }
    if let Some(n) = expected.expected_artifacts.gpu_samples {
        assert_eq!(
            analysis.artifacts_summary.gpu_sample_count, n,
            "{} gpu_sample_count",
            expected.name
        );
    }
    if let Some(n) = expected.expected_artifacts.frames {
        assert_eq!(
            analysis.artifacts_summary.frame_event_count, n,
            "{} frame_event_count",
            expected.name
        );
    }
    if let Some(n) = expected.expected_artifacts.block_io_events {
        assert_eq!(
            analysis.artifacts_summary.block_io_event_count, n,
            "{} block_io_event_count",
            expected.name
        );
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
    assert_fixture(ExpectedFixture {
        name: "cpu_pressure",
        expected_primary: Some(StutterCause::CpuPressureCandidate),
        accepted_confidence: &[Confidence::Medium, Confidence::High],
        expected_quality: DataQualityLevel::High,
        evidence_substrings: &["high CPU PSI"],
        expected_artifacts: ExpectedArtifacts {
            spikes: Some(3),
            intervals: Some(1),
            ..Default::default()
        },
    });
}

#[test]
fn validation_corpus_block_io_stall() {
    assert_fixture(ExpectedFixture {
        name: "block_io_stall",
        expected_primary: Some(StutterCause::BlockIoCandidate),
        accepted_confidence: &[Confidence::Medium, Confidence::High],
        expected_quality: DataQualityLevel::High,
        evidence_substrings: &["block I/O"],
        expected_artifacts: ExpectedArtifacts {
            spikes: Some(3),
            intervals: Some(1),
            block_io_events: Some(1),
            ..Default::default()
        },
    });
}

#[test]
fn validation_corpus_irq_heavy() {
    assert_fixture(ExpectedFixture {
        name: "irq_heavy",
        expected_primary: Some(StutterCause::IrqDelayCandidate),
        accepted_confidence: &[Confidence::Medium, Confidence::High],
        expected_quality: DataQualityLevel::High,
        evidence_substrings: &["IRQ"],
        expected_artifacts: ExpectedArtifacts {
            spikes: Some(3),
            intervals: Some(1),
            irq_events: Some(1),
            ..Default::default()
        },
    });
}

#[test]
fn validation_corpus_gpu_bound_clean_cpu_has_gpu_candidate() {
    let analysis = assert_fixture(ExpectedFixture {
        name: "gpu_bound_clean_cpu",
        expected_primary: Some(StutterCause::GpuBoundCandidate),
        accepted_confidence: &[Confidence::Low, Confidence::Medium, Confidence::High],
        expected_quality: DataQualityLevel::High,
        evidence_substrings: &["GPU busy"],
        expected_artifacts: ExpectedArtifacts {
            spikes: Some(3),
            intervals: Some(1),
            gpu_samples: Some(1),
            frames: Some(3),
            ..Default::default()
        },
    });

    assert_candidate_contains(&analysis, StutterCause::GpuBoundCandidate, &["GPU busy"]);
}

#[test]
fn validation_corpus_clean_run_is_high_quality_without_false_diagnosis() {
    let analysis = assert_fixture(ExpectedFixture {
        name: "clean_run",
        expected_primary: None,
        accepted_confidence: &[],
        expected_quality: DataQualityLevel::High,
        evidence_substrings: &[],
        expected_artifacts: ExpectedArtifacts {
            spikes: Some(0),
            intervals: Some(2),
            irq_events: Some(0),
            gpu_samples: Some(0),
            frames: Some(0),
            block_io_events: Some(0),
        },
    });

    assert!(analysis.data_quality.validation_errors.is_empty());
    assert!(analysis.cluster_analysis.clusters.is_empty());
}

#[test]
fn validation_corpus_truncated_drop_counters_is_not_high_quality() {
    let analysis = assert_fixture(ExpectedFixture {
        name: "truncated_drop_counters",
        expected_primary: None,
        accepted_confidence: &[],
        expected_quality: DataQualityLevel::Medium,
        evidence_substrings: &[],
        expected_artifacts: ExpectedArtifacts {
            spikes: Some(1),
            intervals: Some(1),
            ..Default::default()
        },
    });

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
    let analysis = assert_fixture(ExpectedFixture {
        name: "reused_tid_no_contamination",
        expected_primary: None,
        accepted_confidence: &[],
        expected_quality: DataQualityLevel::High,
        evidence_substrings: &[],
        expected_artifacts: ExpectedArtifacts {
            spikes: Some(0),
            intervals: Some(2),
            ..Default::default()
        },
    });

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
    let analysis = build_fixture_analysis("old_schema_warning");

    assert!(matches!(
        analysis.data_quality.level,
        DataQualityLevel::Medium | DataQualityLevel::High
    ));
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
fn regenerate_validation_corpus() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("runs");
    test_fixture_builder::write_validation_corpus(&root)
        .unwrap_or_else(|err| panic!("failed to regenerate validation corpus: {err:#}"));
}
