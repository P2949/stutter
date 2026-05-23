//! Tests extracted from the parent module to keep production module size bounded.

use super::{super::RecordingCounters, *};

#[test]
fn write_json_rejects_path_without_file_name() {
    let err = write_json(
        PathBuf::from("/"),
        &serde_json::json!({}),
        &mut SyncTracker::default(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("no file name"));
}

#[test]
fn spike_point_preserves_switch_prev_context() {
    let stats = crate::metrics::TaskStats::new(42, "t".to_owned(), 0);
    let spike = crate::metrics::SpikeRecord {
        latency_ns: 100,
        cpu: 1,
        wakeup_target_cpu: 0,
        prio: 0,
        wakeup_ns: 10,
        switch_ns: 110,
        switch_prev_pid: 99,
        switch_prev_state: 1,
        switch_prev_state_label: "voluntary_sleep_interruptible".to_owned(),
        ..crate::metrics::SpikeRecord::default()
    };

    let rec = recorded_spike(&stats, &spike);
    assert_eq!(rec.switch_prev_pid, 99);
    assert_eq!(rec.switch_prev_state, 1);
}

#[test]
fn recording_warnings_include_intervals_dropped() {
    let recorder = LiveRecorder {
        counters: RecordingCounters {
            intervals_dropped: 3,
            ..Default::default()
        },
        ..Default::default()
    };

    let warnings = recording_warnings(&recorder);

    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("3 interval record(s) were dropped"));
    assert!(warnings[0].contains("--retain-intervals"));
}

#[test]
fn recording_warnings_include_spike_events_dropped() {
    let recorder = LiveRecorder {
        counters: RecordingCounters {
            spike_events_dropped_count: 2,
            ..Default::default()
        },
        ..Default::default()
    };

    let warnings = recording_warnings(&recorder);

    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("2 spike event record(s)"));
}

#[test]
fn recording_warnings_include_event_stream_write_errors() {
    let recorder = LiveRecorder {
        counters: RecordingCounters {
            event_stream_write_errors: 4,
            ..Default::default()
        },
        ..Default::default()
    };

    let warnings = recording_warnings(&recorder);

    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("4 event stream write error(s)"));
    assert!(warnings[0].contains("incomplete"));
}

#[test]
fn recording_warnings_include_all_recording_problems() {
    let recorder = LiveRecorder {
        counters: RecordingCounters {
            intervals_dropped: 1,
            spike_events_dropped_count: 2,
            event_stream_write_errors: 3,
            ..Default::default()
        },
        ..Default::default()
    };

    let warnings = recording_warnings(&recorder);

    assert_eq!(warnings.len(), 3);
}

#[test]
fn recording_warnings_empty_for_clean_recorder() {
    let recorder = LiveRecorder::default();

    let warnings = recording_warnings(&recorder);

    assert!(warnings.is_empty());
}

fn temp_dir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "stutter-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    dir
}

#[test]
fn finalize_recording_persists_full_final_foreground_identity() {
    let dir = temp_dir("final-foreground-identity");
    fs::create_dir_all(&dir).unwrap();

    let recorder = LiveRecorder {
        run: Some(RecordingRun {
            run_name: Some("final-foreground-identity".to_owned()),
            run_dir: dir.clone(),
            started_at: SystemTime::now(),
            started_instant: Instant::now(),
            monotonic_start_ns: Some(1),
            mangohud_start_offset: None,
            mangohud_first_frame_monotonic_ns: None,
            mangohud_first_frame_raw_elapsed_ms: None,
        }),
        counters: RecordingCounters {
            foreground_event_count: 1,
            ..RecordingCounters::default()
        },
        ..LiveRecorder::default()
    };
    let config = MonitorConfig::default();
    let tasks = crate::tasks::TaskTracker::default();
    let foreground = ForegroundEvent {
        elapsed_ms: 1_000,
        source: crate::foreground::ForegroundSource::Sway,
        status: crate::foreground::ForegroundProviderStatus::Available,
        pid: Some(159447),
        app_id: Some("steam_app_379430".to_owned()),
        class: Some("steam_app_379430".to_owned()),
        title: None,
        window_id: Some("163".to_owned()),
        workspace: Some("5".to_owned()),
        confidence: 0.95,
        stale_ms: Some(500),
        reason: "focused Sway node from swaymsg get_tree".to_owned(),
    };

    finalize_recording(FinalizeRecordingInput {
        recorder: &recorder,
        config: &config,
        tree_pids: &[],
        stop_reason: "test",
        tasks: &tasks,
        frame_events: &[],
        block_io_correlation_basis: crate::ebpf_loader::BlockIoCorrelationBasis::RequestPointer
            .as_str()
            .to_owned(),
        block_io_correlation_confidence:
            crate::ebpf_loader::BlockIoCorrelationBasis::RequestPointer
                .confidence()
                .to_owned(),
        native_cgroup_filter: crate::ebpf_loader::NativeCgroupFilterStatus::default(),
        drop_counters: crate::ebpf_loader::DropCountersSnapshot::default(),
        cpu_perf_status: None,
        focus_mode: None,
        final_focus_kind: None,
        focus_switch_count: 0,
        current_focus: None,
        final_foreground_event: Some(foreground),
    })
    .unwrap();

    let session: SessionFile =
        serde_json::from_str(&fs::read_to_string(dir.join("session.json")).unwrap()).unwrap();
    let metadata: MetadataFile =
        serde_json::from_str(&fs::read_to_string(dir.join("metadata.json")).unwrap()).unwrap();

    for core in [&session.core, &metadata.core] {
        assert_eq!(core.final_foreground_pid, Some(159447));
        assert_eq!(
            core.final_foreground_app_id.as_deref(),
            Some("steam_app_379430")
        );
        assert_eq!(
            core.final_foreground_class.as_deref(),
            Some("steam_app_379430")
        );
        assert_eq!(core.final_foreground_status.as_deref(), Some("available"));
        assert_eq!(core.final_foreground_window_id.as_deref(), Some("163"));
        assert_eq!(core.final_foreground_workspace.as_deref(), Some("5"));
        assert_eq!(core.final_foreground_confidence, Some(0.95));
        assert_eq!(core.final_foreground_stale_ms, Some(500));
        assert_eq!(
            core.final_foreground_reason.as_deref(),
            Some("focused Sway node from swaymsg get_tree")
        );
    }

    fs::remove_dir_all(dir).ok();
}

#[test]
fn sync_tracker_tracks_parent_once_for_same_directory() {
    let mut tracker = SyncTracker::default();

    tracker.mark_parent_for_test(Path::new("run-a/session.json"));
    tracker.mark_parent_for_test(Path::new("run-a/metadata.json"));

    assert_eq!(tracker.synced_dir_count_for_test(), 1);
}

#[test]
fn sync_tracker_tracks_distinct_parent_directories() {
    let mut tracker = SyncTracker::default();

    tracker.mark_parent_for_test(Path::new("run-a/session.json"));
    tracker.mark_parent_for_test(Path::new("run-b/session.json"));

    assert_eq!(tracker.synced_dir_count_for_test(), 2);
}

#[test]
fn sync_parent_once_does_not_error_for_existing_parent() {
    let dir = temp_dir("sync-tracker");
    fs::create_dir_all(&dir).unwrap();

    let path = dir.join("session.json");
    fs::write(&path, "{}\n").unwrap();

    let mut tracker = SyncTracker::default();
    tracker.sync_parent_once(&path).unwrap();
    tracker
        .sync_parent_once(&dir.join("metadata.json"))
        .unwrap();

    assert_eq!(tracker.synced_dir_count_for_test(), 1);
    fs::remove_dir_all(dir).ok();
}

#[test]
fn live_recorder_writes_foreground_event_to_dedicated_stream() {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "stutter-foreground-event-stream-{}-{}",
        std::process::id(),
        crate::audit::unix_nanos_now()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("foreground_events.json");

    let mut recorder = LiveRecorder::default();
    recorder
        .streams
        .create_stream(&dir, ArtifactKind::ForegroundEvents)
        .unwrap();

    let event = ForegroundEvent {
        elapsed_ms: 42,
        source: crate::foreground::ForegroundSource::X11,
        status: crate::foreground::ForegroundProviderStatus::Available,
        pid: Some(1000),
        app_id: Some("Navigator".to_owned()),
        class: Some("Firefox".to_owned()),
        title: None,
        window_id: Some("0x1200007".to_owned()),
        workspace: None,
        confidence: 0.90,
        stale_ms: None,
        reason: "active X11 window from xprop".to_owned(),
    };

    recorder.write_foreground_event(event.clone()).unwrap();
    recorder.streams.finish_all().unwrap();

    assert_eq!(recorder.counters.foreground_event_count, 1);
    assert_eq!(
        recorder.last_foreground_event.as_ref().unwrap().pid,
        Some(1000)
    );

    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("\"source\":\"x11\""));
    assert!(text.contains("\"pid\":1000"));
    assert!(!text.contains("focus"));

    std::fs::remove_dir_all(dir).ok();
}
