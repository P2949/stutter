use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use super::{engine::*, models::*, scanner::*};
use crate::{diagnosis::StutterCause, report::DataQualityLevel};

fn temp_dir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "stutter-advisor-test-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn report_for(causes: &[StutterCause], quality: DataQualityLevel) -> AdvisorReport {
    build_advisor_report_from_evidence(AdvisorEvidenceInput {
        run: Path::new("/tmp/run"),
        data_quality: quality,
        causes,
        cause_evidence: &[],
        profiles: Some(Path::new("profiles.toml")),
        signal_availability: AdvisorSignalAvailability {
            has_hwmon: false,
            has_irq: false,
            has_block_io: false,
        },
        tree_pid: Some(42),
    })
}

#[test]
fn low_data_quality_blocks_tuning_recommendation() {
    let report = report_for(
        &[StutterCause::GameThreadSchedulerDelay],
        DataQualityLevel::Low,
    );

    assert_eq!(report.verdict, AdvisorVerdict::CollectMoreData);
    assert!(
        !report
            .recommendations
            .iter()
            .any(|rec| rec.title.contains("profile tuning"))
    );
}

#[test]
fn compositor_scheduler_delay_recommends_profile_tuning() {
    let report = report_for(
        &[StutterCause::CompositorSchedulerDelay],
        DataQualityLevel::High,
    );

    assert_eq!(report.verdict, AdvisorVerdict::TryProfileTuning);
    assert!(report.recommendations[0].suggested_commands[0].contains("stutter tune --tree-pid 42"));
}

#[test]
fn gpu_bound_warns_cpu_affinity_may_not_help() {
    let report = report_for(&[StutterCause::GpuBoundCandidate], DataQualityLevel::High);

    assert_eq!(report.verdict, AdvisorVerdict::InvestigateNonCpuBottleneck);
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.contains("CPU affinity may not help"))
    );
}

#[test]
fn irq_candidate_does_not_suggest_changing_irq_affinity_yet() {
    let report = report_for(&[StutterCause::IrqDelayCandidate], DataQualityLevel::High);

    assert_eq!(report.verdict, AdvisorVerdict::InvestigateNonCpuBottleneck);
    assert!(
        report
            .recommendations
            .iter()
            .flat_map(|rec| rec.suggested_commands.iter())
            .all(|command| !command.contains("irq affinity"))
    );
    assert!(
        report.recommendations[0]
            .safety_note
            .contains("do not change IRQ affinity yet")
    );
}

#[test]
fn recommendation_rationale_includes_structured_evidence() {
    let cause_evidence = vec![AdvisorCauseEvidence {
        cause: StutterCause::IrqDelayCandidate,
        messages: vec!["IRQ 146 on CPU 2 overlapped with the game thread for 55ms".to_owned()],
    }];
    let report = build_advisor_report_from_evidence(AdvisorEvidenceInput {
        run: Path::new("/tmp/run"),
        data_quality: DataQualityLevel::High,
        causes: &[StutterCause::IrqDelayCandidate],
        cause_evidence: &cause_evidence,
        profiles: Some(Path::new("profiles.toml")),
        signal_availability: AdvisorSignalAvailability {
            has_hwmon: false,
            has_irq: true,
            has_block_io: false,
        },
        tree_pid: Some(42),
    });

    assert!(
        report.recommendations[0]
            .rationale
            .contains("IRQ 146 on CPU 2")
    );
}

#[test]
fn unknown_result_suggests_more_data() {
    let report = report_for(&[StutterCause::Unknown], DataQualityLevel::High);

    assert_eq!(report.verdict, AdvisorVerdict::CollectMoreData);
}

#[test]
fn watch_scanner_finds_completed_run_dirs() {
    let dir = temp_dir("scanner-finds");
    let run = dir.join("run-a");
    fs::create_dir_all(&run).unwrap();
    fs::write(run.join("session.json"), "{}").unwrap();

    let runs = completed_run_dirs_with_min_age(&dir, &BTreeSet::new(), Duration::ZERO).unwrap();

    assert_eq!(runs, vec![run]);
    fs::remove_dir_all(dir).ok();
}

#[test]
fn watch_scanner_ignores_dirs_without_session() {
    let dir = temp_dir("scanner-ignores");
    fs::create_dir_all(dir.join("run-a")).unwrap();

    let runs = completed_run_dirs_with_min_age(&dir, &BTreeSet::new(), Duration::ZERO).unwrap();

    assert!(runs.is_empty());
    fs::remove_dir_all(dir).ok();
}

#[test]
fn watch_scanner_skips_recently_modified_session() {
    let dir = temp_dir("scanner-recent");
    let run = dir.join("run-a");
    fs::create_dir_all(&run).unwrap();
    fs::write(run.join("session.json"), "{}").unwrap();

    let runs =
        completed_run_dirs_with_min_age(&dir, &BTreeSet::new(), Duration::from_secs(2)).unwrap();

    assert!(runs.is_empty());
    fs::remove_dir_all(dir).ok();
}

#[test]
fn watch_scanner_does_not_process_same_path_twice() {
    let dir = temp_dir("scanner-processed");
    let run = dir.join("run-a");
    fs::create_dir_all(&run).unwrap();
    fs::write(run.join("session.json"), "{}").unwrap();
    let processed = BTreeSet::from([run.clone()]);

    let runs = completed_run_dirs_with_min_age(&dir, &processed, Duration::ZERO).unwrap();

    assert!(runs.is_empty());
    fs::remove_dir_all(dir).ok();
}

#[test]
fn gpu_and_scheduler_both_produce_recommendations() {
    let report = report_for(
        &[
            StutterCause::GpuBoundCandidate,
            StutterCause::GameThreadSchedulerDelay,
        ],
        DataQualityLevel::High,
    );

    assert_eq!(report.verdict, AdvisorVerdict::InvestigateNonCpuBottleneck);
    assert!(
        report
            .recommendations
            .iter()
            .any(|r| r.title.contains("non-CPU"))
    );
    assert!(
        report
            .recommendations
            .iter()
            .any(|r| r.title.contains("profile tuning"))
    );
}
