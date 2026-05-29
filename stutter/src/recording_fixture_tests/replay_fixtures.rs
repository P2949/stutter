use super::*;

#[test]
fn report_replay_fixture_game_thread_scheduler_delay() {
    let dir = temp_run_dir("replay-game-scheduler");
    let spikes = clustered_spikes(TaskClass::Game, "RenderThread", 8_000_000);
    write_base_run(&dir, "game_thread_scheduler_delay", |session| {
        apply_spike_session_fields(session, &spikes);
    });
    write_ndjson(dir.join("spike_events.json"), &spikes);

    let analysis = report::build_report_analysis(&dir, 10, 5, None).unwrap();
    let diagnosis = analysis.cluster_analysis.clusters[0]
        .diagnosis
        .as_ref()
        .unwrap();

    assert_eq!(diagnosis.cause, StutterCause::GameThreadSchedulerDelay);
    assert!(matches!(
        diagnosis.confidence,
        Confidence::High | Confidence::Medium
    ));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn report_replay_fixture_compositor_scheduler_delay() {
    let dir = temp_run_dir("replay-compositor-scheduler");
    let spikes = clustered_spikes(TaskClass::Compositor, "sway", 6_000_000);
    write_base_run(&dir, "compositor_scheduler_delay", |session| {
        apply_spike_session_fields(session, &spikes);
    });
    write_ndjson(dir.join("spike_events.json"), &spikes);

    let analysis = report::build_report_analysis(&dir, 10, 5, None).unwrap();
    let diagnosis = analysis.cluster_analysis.clusters[0]
        .diagnosis
        .as_ref()
        .unwrap();

    assert_eq!(diagnosis.cause, StutterCause::CompositorSchedulerDelay);

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn report_replay_fixture_irq_overlap() {
    let dir = temp_run_dir("replay-irq-overlap");
    let spikes = vec![
        spike_event(100, TaskClass::Unknown, "worker-a", 3_000_000, 0),
        spike_event(101, TaskClass::Unknown, "worker-b", 2_500_000, 250_000),
        spike_event(102, TaskClass::Unknown, "worker-c", 2_000_000, 500_000),
    ];
    let irq_events = vec![IrqEventRecord {
        elapsed_ms: Some(100),
        irq: 137,
        cpu: 0,
        enter_ns: 99_000_000,
        exit_ns: 103_000_000,
        duration_ns: 4_000_000,
    }];
    write_base_run(&dir, "irq_overlap", |session| {
        apply_spike_session_fields(session, &spikes);
        session.core.irq_event_count = irq_events.len() as u64;
    });
    write_ndjson(dir.join("spike_events.json"), &spikes);
    write_ndjson(dir.join("irq_events.json"), &irq_events);

    let analysis = report::build_report_analysis(&dir, 10, 5, None).unwrap();
    let diagnosis = analysis.cluster_analysis.clusters[0]
        .diagnosis
        .as_ref()
        .unwrap();

    assert_eq!(diagnosis.cause, StutterCause::IrqDelayCandidate);

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn report_replay_fixture_gpu_bound_candidate() {
    let dir = temp_run_dir("replay-gpu-bound");
    let spikes = vec![
        spike_event(100, TaskClass::Unknown, "worker-a", 3_000_000, 0),
        spike_event(101, TaskClass::Unknown, "worker-b", 2_500_000, 250_000),
        spike_event(102, TaskClass::Unknown, "worker-c", 2_000_000, 500_000),
    ];
    let gpu_samples = vec![GpuSample {
        elapsed_ms: 100,
        gpu_busy_percent: Some(99),
        ..Default::default()
    }];
    write_base_run(&dir, "gpu_bound", |session| {
        apply_spike_session_fields(session, &spikes);
        session.core.gpu_sample_count = gpu_samples.len() as u64;
    });
    write_ndjson(dir.join("spike_events.json"), &spikes);
    write_ndjson(dir.join("gpu_samples.json"), &gpu_samples);

    let analysis = report::build_report_analysis(&dir, 10, 5, None).unwrap();
    let diagnosis = analysis.cluster_analysis.clusters[0]
        .diagnosis
        .as_ref()
        .unwrap();

    assert!(candidate_contains(
        diagnosis,
        StutterCause::GpuBoundCandidate
    ));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn report_replay_fixture_block_io_overlap_candidate() {
    let dir = temp_run_dir("replay-block-io");
    let spikes = vec![
        spike_event(100, TaskClass::Unknown, "worker-a", 3_000_000, 0),
        spike_event(101, TaskClass::Unknown, "worker-b", 2_500_000, 250_000),
        spike_event(102, TaskClass::Unknown, "worker-c", 2_000_000, 500_000),
    ];
    let io_events = vec![BlockIoRecord {
        elapsed_ms: 100,
        tid: 100.into(),
        correlation_basis: std::borrow::Cow::Borrowed("request-pointer"),
        dev: 1,
        nr_sector: 8,
        sector: 2048,
        duration_ns: 8_000_000,
        timestamp_ns: 102_000_000,
        rwbs: "R".to_owned(),
    }];
    write_base_run(&dir, "block_io_overlap", |session| {
        apply_spike_session_fields(session, &spikes);
        session.core.block_io_event_count = io_events.len() as u64;
        session.core.block_io_correlation_basis = "request-pointer".to_owned();
    });
    write_ndjson(dir.join("spike_events.json"), &spikes);
    write_ndjson(dir.join("io_events.json"), &io_events);

    let analysis = report::build_report_analysis(&dir, 10, 5, None).unwrap();
    let diagnosis = analysis.cluster_analysis.clusters[0]
        .diagnosis
        .as_ref()
        .unwrap();

    assert!(candidate_contains(
        diagnosis,
        StutterCause::BlockIoCandidate
    ));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn report_replay_fixture_cpu_psi_pressure_candidate() {
    let dir = temp_run_dir("replay-cpu-psi");
    let spikes = vec![
        spike_event(100, TaskClass::Unknown, "worker-a", 3_000_000, 0),
        spike_event(101, TaskClass::Unknown, "worker-b", 2_500_000, 250_000),
        spike_event(102, TaskClass::Unknown, "worker-c", 2_000_000, 500_000),
    ];
    let intervals = vec![IntervalRecord {
        elapsed_ms: 100,
        task: 100,
        active: true,
        class: TaskClass::Unknown,
        comm: "worker-a".to_owned(),
        process_pid: Some(100),
        process_comm: "worker-a".into(),
        samples: 10,
        stored_samples: 10,
        cpu_psi_some: 80.0,
        percentile_scope: "exact".to_owned(),
        ..Default::default()
    }];
    write_base_run(&dir, "cpu_psi_pressure", |session| {
        apply_spike_session_fields(session, &spikes);
        session.core.interval_record_count = intervals.len() as u64;
    });
    write_ndjson(dir.join("spike_events.json"), &spikes);
    write_ndjson(dir.join("interval.json"), &intervals);

    let analysis = report::build_report_analysis(&dir, 10, 5, None).unwrap();
    let diagnosis = analysis.cluster_analysis.clusters[0]
        .diagnosis
        .as_ref()
        .unwrap();

    assert!(candidate_contains(
        diagnosis,
        StutterCause::CpuPressureCandidate
    ));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn report_replay_fixture_low_quality_missing_optional_files() {
    let dir = temp_run_dir("replay-low-quality-missing");
    fs::create_dir_all(&dir).unwrap();
    let session = base_session("low_quality_missing_optional_files");
    write_json_pretty(dir.join("session.json"), &session);

    let analysis = report::build_report_analysis(&dir, 10, 5, None).unwrap();

    assert!(matches!(
        analysis.data_quality.level,
        DataQualityLevel::Medium | DataQualityLevel::Low
    ));
    assert!(
        analysis
            .data_quality
            .missing_optional_files
            .iter()
            .any(|file| file == "metadata.json")
    );

    let _ = fs::remove_dir_all(dir);
}
