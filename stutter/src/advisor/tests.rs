use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use super::{engine::*, models::*, scanner::*};
use crate::{
    diagnosis::{
        Confidence, Diagnosis, DiagnosisCandidate, EvidenceItem, EvidenceKind, StutterCause,
    },
    irq_inspect::IrqLine,
    process_tree::TaskClass,
    recorder::{SessionFile, SessionTask},
    report::{
        CrossGpuFenceSummary, DataQualityLevel, DirectScanoutSummary, DisplayPathDiagnosisSummary,
        DmaBufPathSummary, DrmFenceTimingSummary, FocusReportSummary, ForegroundReportSummary,
        FramePacingSummary, GpuEngineActivitySummary, KmsTimingSummary, PressureTimelineSummary,
        ReportAnalysisJson, RuntimeSliceAnalysisSummary, SpikeClusterAnalysis, SpikeClusterSource,
        WaylandPresentationSummary,
    },
    session_io,
    spike::{SpikeCluster, SpikePoint},
};

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
        irq_inventory: &[],
        irq_affinity_overlaps: &[],
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
            .contains("inspect IRQ affinity")
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
        irq_inventory: &[],
        irq_affinity_overlaps: &[],
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

#[test]
fn irq_candidate_uses_irq_inventory_for_specific_recommendation() {
    let irq_lines = vec![crate::irq_inspect::IrqLine {
        irq: "146".to_owned(),
        counts_by_cpu: vec![0, 10],
        total: 10,
        kind: "PCI-MSI".to_owned(),
        name: "524288-edge amdgpu".to_owned(),
        raw: "146: 0 10 PCI-MSI 524288-edge amdgpu".to_owned(),
    }];

    let cause_evidence = vec![AdvisorCauseEvidence {
        cause: StutterCause::IrqDelayCandidate,
        messages: vec![
            "IRQ 146 (524288-edge amdgpu, class=Gpu, cpu=1) active during spike".to_owned(),
        ],
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
        irq_inventory: &irq_lines,
        irq_affinity_overlaps: &[],
    });

    let recommendation = &report.recommendations[0];

    assert!(recommendation.title.contains("specific IRQ"));
    assert!(recommendation.rationale.contains("IRQ 146"));
    assert!(recommendation.rationale.contains("amdgpu"));
    assert!(
        recommendation
            .suggested_commands
            .iter()
            .any(|command| command.contains("146"))
    );
}

#[test]
fn irq_candidate_reports_target_affinity_overlap_when_known() {
    let irq_lines = vec![crate::irq_inspect::IrqLine {
        irq: "146".to_owned(),
        counts_by_cpu: vec![0, 0, 10, 0],
        total: 10,
        kind: "PCI-MSI".to_owned(),
        name: "amdgpu".to_owned(),
        raw: "146: 0 0 10 0 PCI-MSI amdgpu".to_owned(),
    }];

    let overlaps = vec![AdvisorIrqAffinityOverlap {
        irq: 146,
        irq_cpu: 2,
        irq_name: "amdgpu".to_owned(),
        irq_class: crate::irq_inspect::IrqDeviceClass::Gpu,
        overlapping_tasks: vec![AdvisorTargetAffinityOverlap {
            task: 1234,
            comm: "RenderThread".to_owned(),
            class: crate::process_tree::TaskClass::GameRenderThread,
            allowed_cpus: "0-3".to_owned(),
        }],
    }];

    let cause_evidence = vec![AdvisorCauseEvidence {
        cause: StutterCause::IrqDelayCandidate,
        messages: vec![
            "IRQ 146 (amdgpu, class=Gpu, cpu=2) active during spike (max duration 3.000ms)"
                .to_owned(),
        ],
    }];

    let report = build_advisor_report_from_evidence(AdvisorEvidenceInput {
        run: Path::new("/tmp/run"),
        data_quality: DataQualityLevel::High,
        causes: &[StutterCause::IrqDelayCandidate],
        cause_evidence: &cause_evidence,
        profiles: None,
        signal_availability: AdvisorSignalAvailability {
            has_hwmon: false,
            has_irq: true,
            has_block_io: false,
        },
        tree_pid: Some(999),
        irq_inventory: &irq_lines,
        irq_affinity_overlaps: &overlaps,
    });

    let recommendation = report
        .recommendations
        .iter()
        .find(|recommendation| recommendation.title == "Inspect specific IRQ affinity candidate")
        .expect("IRQ-specific recommendation should exist");

    assert!(recommendation.rationale.contains("IRQ 146"));
    assert!(recommendation.rationale.contains("amdgpu"));
    assert!(recommendation.rationale.contains("CPU 2"));
    assert!(
        recommendation
            .rationale
            .contains("recorded target affinity 0-3")
    );
    assert!(recommendation.rationale.contains("RenderThread"));
    assert!(recommendation.rationale.contains("TID 1234"));
    assert!(
        recommendation
            .rationale
            .contains("moving IRQ 146 away from CPU 2")
    );
}

#[test]
fn irq_candidate_mentions_affinity_overlap_unavailable_when_no_task_mask_matches() {
    let irq_lines = vec![crate::irq_inspect::IrqLine {
        irq: "146".to_owned(),
        counts_by_cpu: vec![0, 0, 10, 0],
        total: 10,
        kind: "PCI-MSI".to_owned(),
        name: "amdgpu".to_owned(),
        raw: "146: 0 0 10 0 PCI-MSI amdgpu".to_owned(),
    }];

    let cause_evidence = vec![AdvisorCauseEvidence {
        cause: StutterCause::IrqDelayCandidate,
        messages: vec![
            "IRQ 146 (amdgpu, class=Gpu, cpu=2) active during spike (max duration 3.000ms)"
                .to_owned(),
        ],
    }];

    let report = build_advisor_report_from_evidence(AdvisorEvidenceInput {
        run: Path::new("/tmp/run"),
        data_quality: DataQualityLevel::High,
        causes: &[StutterCause::IrqDelayCandidate],
        cause_evidence: &cause_evidence,
        profiles: None,
        signal_availability: AdvisorSignalAvailability {
            has_hwmon: false,
            has_irq: true,
            has_block_io: false,
        },
        tree_pid: Some(999),
        irq_inventory: &irq_lines,
        irq_affinity_overlaps: &[],
    });

    let recommendation = report
        .recommendations
        .iter()
        .find(|recommendation| recommendation.title == "Inspect specific IRQ affinity candidate")
        .expect("IRQ-specific recommendation should exist");

    assert!(recommendation.rationale.contains("CPU 2"));
    assert!(
        recommendation
            .rationale
            .contains("Recorded target affinity overlap was unavailable")
    );
}

#[test]
fn analysis_builder_derives_irq_affinity_overlap_from_recorded_task_mask() {
    let irq_lines = vec![IrqLine {
        irq: "146".to_owned(),
        counts_by_cpu: vec![0, 0, 10, 0],
        total: 10,
        kind: "PCI-MSI".to_owned(),
        name: "amdgpu".to_owned(),
        raw: "146: 0 0 10 0 PCI-MSI amdgpu".to_owned(),
    }];

    let mut session = SessionFile::default();
    session.core.metadata.irq_lines = irq_lines.clone();
    session.tasks.push(SessionTask {
        task: 1234,
        active: true,
        class: TaskClass::GameRenderThread,
        comm: "RenderThread".to_owned(),
        allowed_cpus: Some("0-3".to_owned()),
        ..Default::default()
    });

    let diagnosis = Diagnosis {
        cause: StutterCause::IrqDelayCandidate,
        confidence: Confidence::Medium,
        secondary_causes: Vec::new(),
        evidence: Vec::new(),
        missing_evidence: Vec::new(),
        primary: None,
        candidates: vec![DiagnosisCandidate {
            cause: StutterCause::IrqDelayCandidate,
            score: 2.0,
            confidence: Confidence::Medium,
            evidence: vec![EvidenceItem {
                kind: EvidenceKind::IrqOverlap,
                strength: 1.0,
                message: "IRQ 146 (amdgpu, class=Gpu, cpu=2) active during spike".to_owned(),
                timestamp_ms: Some(100),
                start_ns: None,
                end_ns: None,
            }],
        }],
        candidate_rejections: Vec::new(),
        summary: "IRQ candidate".to_owned(),
    };

    let cluster = SpikeCluster {
        points: vec![SpikePoint {
            task: 1234,
            class: TaskClass::GameRenderThread,
            comm: "RenderThread".to_owned(),
            ..Default::default()
        }],
        distinct_tasks: 1,
        diagnosis: Some(diagnosis),
        ..Default::default()
    };

    let validation = session_io::RunValidationReport::default();
    let analysis = ReportAnalysisJson {
        session: session.clone(),
        cluster_analysis: SpikeClusterAnalysis {
            source: SpikeClusterSource::TopSpikesFallback,
            source_count: 1,
            clusters: vec![cluster],
        },
        frame_diagnoses: Vec::new(),
        frame_pacing: FramePacingSummary::default(),
        pressure_timeline: PressureTimelineSummary::default(),
        runtime_slices: RuntimeSliceAnalysisSummary::default(),
        diagnosis_thresholds: Vec::new(),
        artifacts_summary: crate::report::artifacts_summary_from_session(&session),
        data_quality: crate::report::data_quality_summary(&session, &validation),
        focus_summary: FocusReportSummary::default(),
        foreground_summary: ForegroundReportSummary::default(),
        kms_timing: KmsTimingSummary::default(),
        drm_fence_timing: DrmFenceTimingSummary::default(),
        cross_gpu_fence: CrossGpuFenceSummary::default(),
        wayland_presentation: WaylandPresentationSummary::default(),
        direct_scanout: DirectScanoutSummary::default(),
        dmabuf_path: DmaBufPathSummary::default(),
        gpu_engine_activity: GpuEngineActivitySummary::default(),
        display_path_diagnosis: DisplayPathDiagnosisSummary::default(),
    };

    let overlaps = irq_affinity_overlaps_from_analysis_for_tests(&analysis, &irq_lines);

    assert_eq!(overlaps.len(), 1);
    assert_eq!(overlaps[0].irq, 146);
    assert_eq!(overlaps[0].irq_cpu, 2);
    assert_eq!(overlaps[0].overlapping_tasks[0].task, 1234);
    assert_eq!(overlaps[0].overlapping_tasks[0].allowed_cpus, "0-3");
}

#[test]
fn gpu_candidate_uses_power_limit_and_fence_evidence_in_recommendation() {
    let cause_evidence = vec![AdvisorCauseEvidence {
        cause: StutterCause::GpuBoundCandidate,
        messages: vec![
            "GPU power limit active near spike (reason: power_cap)".to_owned(),
            "DRM fence wait near spike: role=render driver=amdgpu comm=Game.exe duration=3ms"
                .to_owned(),
        ],
    }];

    let report = build_advisor_report_from_evidence(AdvisorEvidenceInput {
        run: Path::new("/tmp/run"),
        data_quality: DataQualityLevel::High,
        causes: &[StutterCause::GpuBoundCandidate],
        cause_evidence: &cause_evidence,
        profiles: Some(Path::new("profiles.toml")),
        signal_availability: AdvisorSignalAvailability {
            has_hwmon: true,
            has_irq: false,
            has_block_io: false,
        },
        tree_pid: Some(42),
        irq_inventory: &[],
        irq_affinity_overlaps: &[],
    });

    let recommendation = &report.recommendations[0];

    assert!(recommendation.rationale.contains("power limit"));
    assert!(recommendation.rationale.contains("DRM fence wait"));
}
