//! Regression coverage for report rendering, artifact correlation, and report diffs.

use super::{support::*, *};

#[test]
fn report_reads_recorded_session_and_spike_events() {
    let dir = temp_test_dir("report-smoke");
    fs::create_dir_all(&dir).unwrap();

    let recording = RecordingRun {
        run_name: Some("report-test".to_owned()),
        run_dir: dir.clone(),
        started_at: UNIX_EPOCH,
        started_instant: Instant::now(),
        monotonic_start_ns: Some(1_000_000_000),
        mangohud_start_offset: None,
        mangohud_first_frame_monotonic_ns: None,
        mangohud_first_frame_raw_elapsed_ms: None,
    };
    let config = test_config(vec![7], vec![], Some(Duration::from_secs(1)));
    let active_targets = BTreeMap::from([(
        7,
        task_info(7, 7, "KingdomCome.exe", "RenderThread", TaskClass::Game),
    )]);
    let mut stats = metrics::TaskStats::new(7, "RenderThread".to_owned(), 0);
    stats.apply_task_info(active_targets.get(&7).unwrap());
    stats.session_latency.record(6_000_000);
    stats.top_spikes.push(metrics::SpikeRecord {
        latency_ns: 6_000_000,
        cpu: 0,
        wakeup_target_cpu: 0,
        prio: 120,
        wakeup_ns: 1_010_000_000,
        switch_ns: 1_016_000_000,
        switch_prev_pid: 0,
        switch_prev_state: 0,
        switch_prev_state_label: "".to_owned(),
        ..metrics::SpikeRecord::default()
    });
    let stats_by_task = BTreeMap::from([(7, stats)]);
    let spike_events = vec![SpikeEvent {
        elapsed_ms: Some(16),
        task: 7,
        active: true,
        class: TaskClass::Game,
        process_pid: Some(7),
        process_comm: "KingdomCome.exe".into(),
        comm: "RenderThread".into(),
        cpu: 0,
        wakeup_target_cpu: 0,
        prio: 120,
        latency_ns: 6_000_000,
        wakeup_ns: 1_010_000_000,
        switch_ns: 1_016_000_000,
        ..Default::default()
    }];

    let task_tracker = tasks::TaskTracker {
        active_targets,
        stats_by_task,
        ..Default::default()
    };

    let mut buffer = SpikeEventBuffer::default();
    for spike in spike_events {
        buffer.push(spike);
    }
    let recorder = recorder::LiveRecorder {
        run: Some(recording),
        buffers: recorder::LiveBuffers {
            spike_events: Some(buffer),
            ..Default::default()
        },
        ..Default::default()
    };

    let monitor_config = config.clone();
    recorder::finalize_recording(FinalizeRecordingInput {
        recorder: &recorder,
        config: &monitor_config,
        tree_pids: &config.target.tree_pids,
        stop_reason: "test",
        tasks: &task_tracker,
        frame_events: &[],
        block_io_correlation_basis: "dev+sector".to_owned(),
        block_io_correlation_confidence: "medium".to_owned(),
        drop_counters: DropCountersSnapshot::default(),
        cpu_perf_status: None,
        focus_mode: None,
        final_focus_kind: None,
        focus_switch_count: 0,
        current_focus: None,
        final_foreground_event: None,
    })
    .unwrap();

    crate::report::print_report(crate::report::PrintReportInput {
        path: &dir,
        json: false,
        analysis_json: false,
        json_summary: false,
        top: 10,
        cluster_window_ms: 5,
        filter_class: None,
        flamegraph: None,
    })
    .unwrap();
    crate::report::print_report(crate::report::PrintReportInput {
        path: &dir,
        json: true,
        analysis_json: false,
        json_summary: false,
        top: 10,
        cluster_window_ms: 5,
        filter_class: None,
        flamegraph: None,
    })
    .unwrap();

    fs::remove_dir_all(dir).ok();
}

#[test]
fn report_cluster_output_caps_inline_points() {
    let dir = temp_test_dir("report-cluster-cap");
    fs::create_dir_all(&dir).unwrap();

    let recording = RecordingRun {
        run_name: Some("cluster-cap-test".to_owned()),
        run_dir: dir.clone(),
        started_at: UNIX_EPOCH,
        started_instant: Instant::now(),
        monotonic_start_ns: Some(1_000_000_000),
        mangohud_start_offset: None,
        mangohud_first_frame_monotonic_ns: None,
        mangohud_first_frame_raw_elapsed_ms: None,
    };

    let config = test_config(vec![7], vec![], Some(Duration::from_secs(1)));
    let active_targets: BTreeMap<u32, TaskInfo> = BTreeMap::new();
    let stats_by_task: BTreeMap<u32, metrics::TaskStats> = BTreeMap::new();

    let spike_events = (0..10)
        .map(|idx| SpikeEvent {
            elapsed_ms: Some(idx as u64),
            task: 100 + idx as u32,
            active: true,
            class: TaskClass::Helper,
            process_pid: Some(100 + idx as u32),
            process_comm: format!("proc-{}", idx).into(),
            comm: format!("worker-{}", idx),
            cpu: idx as u32 % 4,
            wakeup_target_cpu: idx as u32 % 4,
            prio: 120,
            latency_ns: 1_000_000 + idx as u64,
            wakeup_ns: 1_000_000_000 + idx as u64 * 100_000,
            switch_ns: 1_001_000_000 + idx as u64 * 100_000,
            ..Default::default()
        })
        .collect::<Vec<_>>();

    let task_tracker = tasks::TaskTracker {
        active_targets,
        stats_by_task,
        ..Default::default()
    };

    let mut buffer = SpikeEventBuffer::default();
    for spike in spike_events.iter().cloned() {
        buffer.push(spike);
    }
    let recorder = recorder::LiveRecorder {
        run: Some(recording),
        buffers: recorder::LiveBuffers {
            spike_events: Some(buffer),
            ..Default::default()
        },
        ..Default::default()
    };

    let monitor_config = config.clone();
    recorder::finalize_recording(FinalizeRecordingInput {
        recorder: &recorder,
        config: &monitor_config,
        tree_pids: &config.target.tree_pids,
        stop_reason: "test",
        tasks: &task_tracker,
        frame_events: &[],
        block_io_correlation_basis: "dev+sector".to_owned(),
        block_io_correlation_confidence: "medium".to_owned(),
        drop_counters: DropCountersSnapshot::default(),
        cpu_perf_status: None,
        focus_mode: None,
        final_focus_kind: None,
        focus_switch_count: 0,
        current_focus: None,
        final_foreground_event: None,
    })
    .unwrap();

    let session = crate::session_io::load_session(&dir).unwrap();

    let artifacts = crate::session_io::RunArtifacts {
        spikes: spike_events,
        ..Default::default()
    };
    let output = render_report_for_test(&session, &artifacts, 5, 10);

    assert!(output.contains("total_spikes=10"));
    assert!(output.contains("shown_points=8"));
    assert!(output.contains("omitted_points=2"));
    assert!(output.contains("100("));
    assert!(output.contains("107("));
    assert!(!output.contains("108("));
    assert!(!output.contains("109("));

    fs::remove_dir_all(dir).ok();
}

#[test]
fn report_correlates_artifacts_with_spike_clusters() {
    let dir = temp_test_dir("report-correlation");
    fs::create_dir_all(&dir).unwrap();
    let session = minimal_session_for_report();
    let spike_events = (0..3)
        .map(|idx| SpikeEvent {
            elapsed_ms: Some(10 + idx as u64),
            task: 10 + idx as u32,
            active: true,
            class: TaskClass::Game,
            process_pid: Some(10 + idx as u32),
            process_comm: "game".to_owned().into(),
            comm: if idx == 0 {
                "RenderThread".to_owned()
            } else {
                format!("worker-{}", idx)
            },
            cpu: idx as u32,
            wakeup_target_cpu: idx as u32,
            prio: 120,
            latency_ns: 1_000_000,
            wakeup_ns: 1_000_000 + idx as u64 * 100,
            switch_ns: 10_000_000 + idx as u64 * 100,
            ..Default::default()
        })
        .collect::<Vec<_>>();
    let artifacts = crate::session_io::RunArtifacts {
        spikes: spike_events,
        scx_events: Vec::new(),
        irq_events: vec![IrqEventRecord {
            elapsed_ms: Some(10),
            irq: 137,
            cpu: 0,
            enter_ns: 9_999_900,
            exit_ns: 10_000_200,
            duration_ns: 300,
        }],
        gpu_samples: vec![GpuSample {
            elapsed_ms: 11,
            gpu_busy_percent: Some(91),
            vram_used_bytes: None,
            vram_total_bytes: None,
            vram_used_percent: None,
            gpu_clock_mhz: Some(2200),
            mem_clock_mhz: Some(1000),
            temp_millidegrees: Some(61000),
            power_microwatts: Some(120_000_000),
            ..GpuSample::default()
        }],
        frame_events: vec![FrameEvent {
            elapsed_ms: 11,
            frametime_ms: 22.5,
        }],
        migration_events: Vec::new(),
        cpu_freq_events: Vec::new(),
        block_io_events: Vec::new(),
        intervals: Vec::new(),
        ..Default::default()
    };

    let output = render_report_for_test(&session, &artifacts, 5, 10);

    assert!(output.contains("irq overlap"));
    assert!(output.contains("irqs=137"));
    assert!(output.contains("gpu near clusters"));
    assert!(output.contains("gpu_busy=91"));
    assert!(output.contains("frame overlap"));
    assert!(output.contains("max_frametime_ms=22.500"));

    fs::remove_dir_all(dir).ok();
}

#[test]
fn report_uses_run_level_block_io_correlation_basis() {
    let dir = temp_test_dir("report-block-io-basis");
    fs::create_dir_all(&dir).unwrap();
    let mut session = minimal_session_for_report();
    session.core.block_io_event_count = 1;
    session.core.block_io_correlation_basis = "request-pointer".to_owned();

    let output =
        render_report_for_test(&session, &crate::session_io::RunArtifacts::default(), 5, 10);

    assert!(output.contains("io_events: 1 (request-pointer correlated (confidence: high))"));
    assert!(!output.contains("block i/o correlation warning"));

    session.core.block_io_correlation_basis = "dev+sector".to_owned();
    let output =
        render_report_for_test(&session, &crate::session_io::RunArtifacts::default(), 5, 10);
    assert!(output.contains(
        "io_events: 1 (dev+sector correlated (advisory, approximate, confidence: medium))"
    ));
    assert!(output.contains("block i/o correlation warning"));

    fs::remove_dir_all(dir).ok();
}
#[test]
fn report_diff_shows_regressions_and_improvements() {
    let dir_a = temp_test_dir("diff-a");
    let dir_b = temp_test_dir("diff-b");
    fs::create_dir_all(&dir_a).unwrap();
    fs::create_dir_all(&dir_b).unwrap();

    let session_a_json = r#"{
        "schema_version": 2,
        "run_name": "run-a",
        "duration_ms": 10000,
        "metadata": {
            "kernel_osrelease": null,
            "kernel_version": null,
            "cpu_online": null,
            "cpu_possible": null,
            "cpu_topology": [],
            "scx_state": null,
            "scx_ops": null,
            "scx_enable_seq": null
        },
        "monotonic_start_ns": 0,
        "started_at": {
            "unix_seconds": 0,
            "unix_nanos": 0,
            "system_time_debug": "test"
        },
        "ended_at": {
            "unix_seconds": 0,
            "unix_nanos": 0,
            "system_time_debug": "test"
        },
        "target_pids_max": 2048,
        "stop_reason": "test",
        "active_target_pids_count": 2,
        "active_expanded_tasks": [],
        "total_targets_tracked": 0,
        "total_events_processed": 0,
        "total_tasks_seen": 0,
        "interval_record_count": 0,
        "intervals_dropped": 0,
        "config": {
            "tree_roots": [],
            "manual_pids": [],
            "include_comm": [],
            "exclude_comm": [],
            "hwmon": false,
            "hwmon_device_prefix": null,
            "hwmon_drm_card": null,
            "hwmon_render_node": null,
            "watch_process": null,
            "watch_process_args": null,
            "persistent": false,
            "csv_stream": null,
            "tui": false,
            "summary_period_ms": 1000,
            "spike_threshold_ns": 5000000,
            "verbose": false
        },
        "tasks": [
            {
                "task": 1,
                "active": true,
                "first_seen_ms": 0,
                "last_seen_ms": 0,
                "removed_ms": null,
                "class": "Game",
                "process_pid": 1,
                "process_comm": "game",
                "comm": "game-thread",
                "latency": {
                    "samples": 100,
                    "stored_samples": 100,
                    "truncated_samples": 0,
                    "percentile_scope": "session",
                    "histogram": [],
                    "min_ns": 100000,
                    "avg_ns": 500000,
                    "p95_ns": 1000000,
                    "p99_ns": 2000000,
                    "max_ns": 5000000,
                    "over_1ms": 10,
                    "over_2ms": 5,
                    "over_5ms": 0
                },
                "cpu": {
                    "busiest_cpu": null,
                    "busiest_cpu_samples": 0,
                    "worst_cpu": null,
                    "worst_cpu_max_ns": 0,
                    "spikiest_cpu": null,
                    "spikiest_cpu_spikes": 0,
                    "per_cpu": []
                },
                "top_spikes": []
            },
            {
                "task": 2,
                "active": true,
                "first_seen_ms": 0,
                "last_seen_ms": 0,
                "removed_ms": null,
                "class": "Helper",
                "process_pid": 1,
                "process_comm": "game",
                "comm": "helper-thread",
                "latency": {
                    "samples": 100,
                    "stored_samples": 100,
                    "truncated_samples": 0,
                    "percentile_scope": "session",
                    "histogram": [],
                    "min_ns": 100000,
                    "avg_ns": 500000,
                    "p95_ns": 1000000,
                    "p99_ns": 2000000,
                    "max_ns": 6000000,
                    "over_1ms": 15,
                    "over_2ms": 5,
                    "over_5ms": 2
                },
                "cpu": {
                    "busiest_cpu": null,
                    "busiest_cpu_samples": 0,
                    "worst_cpu": null,
                    "worst_cpu_max_ns": 0,
                    "spikiest_cpu": null,
                    "spikiest_cpu_spikes": 0,
                    "per_cpu": []
                },
                "top_spikes": []
            }
        ],
        "top_spikes": [],
        "spike_events_retained_count": 0,
        "spike_events_dropped_count": 0,
        "spike_events_truncated": false,
        "drop_counters": {
            "sched_switch": 0,
            "sched_wakeup": 0,
            "scx_runnable": 0,
            "scx_consume": 0,
            "timer_expire_entry": 0,
            "irq_handler_entry": 0,
            "gpu_drm_sched_job": 0,
            "gpu_dma_fence_signaled": 0,
            "sys_enter_read": 0,
            "sys_enter_write": 0,
            "wakeup_data_insert_failed": 0,
            "ringbuf_reserve_failed": 0,
            "irq_start_times_insert_failed": 0,
            "block_start_insert_failed": 0,
            "block_fallback_key_collisions": 0
        },
        "scx_event_count": 0,
        "irq_event_count": 0,
        "migration_event_count": 0,
        "cpu_freq_sample_count": 0,
        "gpu_sample_count": 0,
        "frame_event_count": 0,
        "block_io_event_count": 0,
        "block_io_correlation_basis": "dev+sector"
    }"#;

    let session_b_json = session_a_json
        .replace("\"run-a\"", "\"run-b\"")
        // Game thread max 5ms -> 8ms
        .replace("\"max_ns\": 5000000", "\"max_ns\": 8000000")
        // Game thread over_1ms 10 -> 8
        .replace("\"over_1ms\": 10", "\"over_1ms\": 8")
        // Game thread p99 2ms -> 2.5ms
        .replace("\"p99_ns\": 2000000", "\"p99_ns\": 2500000")
        // Helper thread max 6ms -> 4ms
        .replace("\"max_ns\": 6000000", "\"max_ns\": 4000000");

    fs::write(dir_a.join("session.json"), session_a_json).unwrap();
    fs::write(dir_b.join("session.json"), session_b_json).unwrap();

    let output = crate::report::render_diff_report(&dir_a, &dir_b, 10, None).unwrap();
    println!("DEBUG OUTPUT:\n{}", output);

    assert!(output.contains("regressions"));
    assert!(output.contains("improvements"));
    // Game thread regressed max latency
    assert!(output.contains("max: 5.000ms -> 8.000ms (delta=+3.000ms)"));
    assert!(output.contains("p99_delta=+500.000us"));
    assert!(output.contains("over_1ms_delta=-2"));

    // Helper thread improved max latency
    assert!(output.contains("max: 6.000ms -> 4.000ms (delta=-2.000ms)"));

    // Now test with filter-class
    let output_filtered =
        crate::report::render_diff_report(&dir_a, &dir_b, 10, Some(TaskClass::Game)).unwrap();
    assert!(output_filtered.contains("game-thread"));
    assert!(!output_filtered.contains("helper-thread"));

    fs::remove_dir_all(dir_a).ok();
    fs::remove_dir_all(dir_b).ok();
}
