//! Synthetic, public-example, and autotune replay fixture constructors.

use super::{builders::*, *};

pub(in crate::test_fixture_builder) fn public_clean_baseline_fixture()
-> (SessionFile, FixtureArtifacts) {
    renamed_fixture("clean_baseline", clean_run_fixture())
}

pub(in crate::test_fixture_builder) fn public_game_thread_scheduler_delay_fixture()
-> (SessionFile, FixtureArtifacts) {
    renamed_fixture(
        "game_thread_scheduler_delay_public",
        game_thread_scheduler_delay_fixture(),
    )
}

pub(in crate::test_fixture_builder) fn public_low_quality_truncated_fixture()
-> (SessionFile, FixtureArtifacts) {
    renamed_fixture("low_quality_truncated", truncated_drop_counters_fixture())
}

pub(in crate::test_fixture_builder) fn game_scheduler_pressure_fixture()
-> (SessionFile, FixtureArtifacts) {
    let spikes = vec![
        spike_event(100, TaskClass::Game, "GameMain", 4_000_000, 0),
        spike_event(
            101,
            TaskClass::GameWorkerThread,
            "GameWorker",
            3_500_000,
            250_000,
        ),
        spike_event(102, TaskClass::WineServer, "wineserver", 2_500_000, 500_000),
    ];
    let intervals = vec![
        interval_record_with_class(100, 100, "GameMain", TaskClass::Game, 75.0, 3_000_000),
        interval_record_with_class(
            200,
            101,
            "GameWorker",
            TaskClass::GameWorkerThread,
            70.0,
            2_500_000,
        ),
        interval_record_with_class(
            300,
            102,
            "wineserver",
            TaskClass::WineServer,
            65.0,
            2_000_000,
        ),
    ];

    let mut session = base_session("game_scheduler_pressure");
    session.config.tree_roots = vec![100];
    apply_spike_session_fields(&mut session, &spikes);
    apply_artifact_counts(
        &mut session,
        &FixtureArtifacts {
            spikes,
            intervals,
            ..Default::default()
        },
    )
}

pub(in crate::test_fixture_builder) fn cpu_pressure_fixture() -> (SessionFile, FixtureArtifacts) {
    let spikes = unknown_clustered_spikes(3_000_000);
    let intervals = vec![interval_record(100, 100, "worker-a", 80.0)];

    let mut session = base_session("cpu_pressure");
    apply_spike_session_fields(&mut session, &spikes);
    apply_artifact_counts(
        &mut session,
        &FixtureArtifacts {
            spikes,
            intervals,
            ..Default::default()
        },
    )
}

pub(in crate::test_fixture_builder) fn block_io_stall_fixture() -> (SessionFile, FixtureArtifacts) {
    let spikes = unknown_clustered_spikes(3_000_000);
    let intervals = vec![interval_record(100, 100, "worker-a", 0.0)];
    let block_io_events = vec![BlockIoRecord {
        elapsed_ms: 100,
        tid: 100,
        correlation_basis: Cow::Borrowed("request-pointer"),
        dev: 1,
        nr_sector: 8,
        sector: 2048,
        duration_ns: 8_000_000,
        timestamp_ns: 102_000_000,
        rwbs: "R".to_owned(),
    }];

    let mut session = base_session("block_io_stall");
    apply_spike_session_fields(&mut session, &spikes);
    session.core.block_io_correlation_basis = "request-pointer".to_owned();
    session.config.block_io = true;
    apply_artifact_counts(
        &mut session,
        &FixtureArtifacts {
            spikes,
            intervals,
            block_io_events,
            ..Default::default()
        },
    )
}

pub(in crate::test_fixture_builder) fn irq_heavy_fixture() -> (SessionFile, FixtureArtifacts) {
    let spikes = unknown_clustered_spikes(3_000_000);
    let intervals = vec![interval_record(100, 100, "worker-a", 0.0)];
    let irq_events = vec![IrqEventRecord {
        elapsed_ms: Some(100),
        irq: 137,
        cpu: 0,
        enter_ns: 99_000_000,
        exit_ns: 103_000_000,
        duration_ns: 4_000_000,
    }];

    let mut session = base_session("irq_heavy");
    apply_spike_session_fields(&mut session, &spikes);
    session.config.irq_latency = true;
    session.config.irqs = vec![137];
    apply_artifact_counts(
        &mut session,
        &FixtureArtifacts {
            spikes,
            intervals,
            irq_events,
            ..Default::default()
        },
    )
}

pub(in crate::test_fixture_builder) fn gpu_bound_clean_cpu_fixture()
-> (SessionFile, FixtureArtifacts) {
    let spikes = unknown_clustered_spikes(1_500_000);
    let intervals = vec![interval_record(100, 100, "worker-a", 0.0)];
    let gpu_samples = vec![GpuSample {
        elapsed_ms: 100,
        gpu_busy_percent: Some(99),
        vram_used_bytes: Some(2_000_000_000),
        vram_total_bytes: Some(8_000_000_000),
        vram_used_percent: Some(25),
        gpu_clock_mhz: Some(1800),
        mem_clock_mhz: Some(9500),
        temp_millidegrees: Some(62_000),
        power_microwatts: Some(120_000_000),
        ..GpuSample::default()
    }];
    let frame_events = vec![
        FrameEvent {
            elapsed_ms: 84,
            frametime_ms: 16.6,
        },
        FrameEvent {
            elapsed_ms: 100,
            frametime_ms: 42.0,
        },
        FrameEvent {
            elapsed_ms: 117,
            frametime_ms: 16.7,
        },
    ];

    let mut session = base_session("gpu_bound_clean_cpu");
    apply_spike_session_fields(&mut session, &spikes);
    session.config.hwmon = true;
    session.core.mangohud_first_frame_monotonic_ns = Some(0);
    session.core.mangohud_first_frame_raw_elapsed_ms = Some(0);
    apply_artifact_counts(
        &mut session,
        &FixtureArtifacts {
            spikes,
            intervals,
            gpu_samples,
            frame_events,
            ..Default::default()
        },
    )
}

pub(in crate::test_fixture_builder) fn clean_run_fixture() -> (SessionFile, FixtureArtifacts) {
    let intervals = vec![
        interval_record(100, 100, "main-thread", 0.0),
        interval_record(200, 101, "helper-thread", 0.0),
    ];
    let mut session = base_session("clean_run");
    session.tasks = vec![
        task_for_fixture(100, TaskClass::Game, "main-thread", 25, 900_000),
        task_for_fixture(101, TaskClass::Helper, "helper-thread", 25, 700_000),
    ];
    session.core.active_target_pids_count = 2;
    session.core.active_expanded_tasks = vec![100, 101];
    apply_artifact_counts(
        &mut session,
        &FixtureArtifacts {
            intervals,
            ..Default::default()
        },
    )
}

pub(in crate::test_fixture_builder) fn truncated_drop_counters_fixture()
-> (SessionFile, FixtureArtifacts) {
    let spikes = vec![spike_event(
        100,
        TaskClass::Unknown,
        "worker-a",
        3_000_000,
        0,
    )];
    let intervals = vec![interval_record(100, 100, "worker-a", 0.0)];

    let mut session = base_session("truncated_drop_counters");
    apply_spike_session_fields(&mut session, &spikes);
    session.core.spike_events_truncated = true;
    session.core.spike_events_dropped_count = 7;
    session.core.drop_counters = DropCountersSnapshot {
        wakeup_data_insert_failed: 2,
        wakeup_data_stale_entries: 0,
        wakeup_data_replaced_entries: 0,
        wakeup_data_consumed_read_failed: 0,
        ringbuf_reserve_failed: 1,
        irq_start_times_insert_failed: 0,
        block_start_insert_failed: 0,
        block_fallback_key_collisions: 0,
        cpu_accounting_untracked: 0,
    };
    apply_artifact_counts(
        &mut session,
        &FixtureArtifacts {
            spikes,
            intervals,
            ..Default::default()
        },
    )
}

pub(in crate::test_fixture_builder) fn reused_tid_no_contamination_fixture()
-> (SessionFile, FixtureArtifacts) {
    let intervals = vec![
        interval_record(100, 4242, "old-worker", 0.0),
        interval_record(600, 4242, "new-worker", 0.0),
    ];

    let mut old_task = task_for_fixture(4242, TaskClass::Game, "old-worker", 2, 1_200_000);
    old_task.active = false;
    old_task.first_seen_ms = 0;
    old_task.last_seen_ms = 300;
    old_task.removed_ms = Some(350);
    old_task.process_pid = Some(300);
    old_task.process_comm = "old-app".into();
    old_task.process_starttime_ticks = Some(10_000);
    old_task.task_starttime_ticks = Some(10_100);
    old_task.exe_dev = Some(111);
    old_task.exe_ino = Some(222);

    let mut new_task = task_for_fixture(4242, TaskClass::Helper, "new-worker", 3, 900_000);
    new_task.first_seen_ms = 500;
    new_task.last_seen_ms = 900;
    new_task.process_pid = Some(301);
    new_task.process_comm = "new-app".into();
    new_task.process_starttime_ticks = Some(20_000);
    new_task.task_starttime_ticks = Some(20_100);
    new_task.exe_dev = Some(333);
    new_task.exe_ino = Some(444);

    let mut session = base_session("reused_tid_no_contamination");
    session.tasks = vec![old_task, new_task];
    session.core.active_target_pids_count = 1;
    session.core.active_expanded_tasks = vec![4242];
    apply_artifact_counts(
        &mut session,
        &FixtureArtifacts {
            intervals,
            ..Default::default()
        },
    )
}

pub(in crate::test_fixture_builder) fn old_schema_warning_fixture()
-> (SessionFile, FixtureArtifacts) {
    let intervals = vec![interval_record(100, 100, "worker-a", 0.0)];
    let mut session = base_session("old_schema_warning");
    session.core.schema_version = SESSION_SCHEMA_VERSION - 1;
    apply_artifact_counts(
        &mut session,
        &FixtureArtifacts {
            intervals,
            ..Default::default()
        },
    )
}

pub(in crate::test_fixture_builder) fn game_thread_scheduler_delay_fixture()
-> (SessionFile, FixtureArtifacts) {
    let spikes = vec![
        spike_event(5101, TaskClass::Game, "Main", 8_500_000, 0),
        spike_event(
            5102,
            TaskClass::GameHelper,
            "RenderThread",
            3_200_000,
            250_000,
        ),
        spike_event(
            5103,
            TaskClass::WineServer,
            "wineserver",
            2_900_000,
            500_000,
        ),
    ];
    let intervals = vec![
        interval_record_with_class(100, 5101, "Main", TaskClass::Game, 4.0, 8_500_000),
        interval_record_with_class(
            100,
            5102,
            "RenderThread",
            TaskClass::GameHelper,
            3.0,
            3_200_000,
        ),
        interval_record_with_class(
            100,
            5103,
            "wineserver",
            TaskClass::WineServer,
            2.0,
            2_900_000,
        ),
    ];
    let gpu_samples = vec![GpuSample {
        elapsed_ms: 100,
        gpu_busy_percent: Some(52),
        vram_used_bytes: Some(3_000_000_000),
        vram_total_bytes: Some(8_000_000_000),
        vram_used_percent: Some(37),
        gpu_clock_mhz: Some(1450),
        mem_clock_mhz: Some(7000),
        temp_millidegrees: Some(57_000),
        power_microwatts: Some(78_000_000),
        ..GpuSample::default()
    }];
    let frame_events = vec![
        FrameEvent {
            elapsed_ms: 84,
            frametime_ms: 16.6,
        },
        FrameEvent {
            elapsed_ms: 100,
            frametime_ms: 54.0,
        },
        FrameEvent {
            elapsed_ms: 117,
            frametime_ms: 16.7,
        },
        FrameEvent {
            elapsed_ms: 134,
            frametime_ms: 16.6,
        },
    ];

    let mut session = base_session("game_thread_scheduler_delay");
    session.config.tree_roots = vec![5100];
    session.config.hwmon = true;
    session.core.mangohud_first_frame_monotonic_ns = Some(0);
    session.core.mangohud_first_frame_raw_elapsed_ms = Some(0);
    apply_spike_session_fields(&mut session, &spikes);
    apply_artifact_counts(
        &mut session,
        &FixtureArtifacts {
            spikes,
            intervals,
            gpu_samples,
            frame_events,
            ..Default::default()
        },
    )
}

pub(in crate::test_fixture_builder) fn compositor_scheduler_delay_fixture()
-> (SessionFile, FixtureArtifacts) {
    let spikes = vec![
        spike_event(5201, TaskClass::Compositor, "kwin_wayland", 9_000_000, 0),
        spike_event(5202, TaskClass::Game, "Main", 1_400_000, 250_000),
        spike_event(
            5203,
            TaskClass::Helper,
            "present-worker",
            1_200_000,
            500_000,
        ),
    ];
    let intervals = vec![
        interval_record_with_class(
            100,
            5201,
            "kwin_wayland",
            TaskClass::Compositor,
            5.0,
            9_000_000,
        ),
        interval_record_with_class(100, 5202, "Main", TaskClass::Game, 2.0, 1_400_000),
        interval_record_with_class(
            100,
            5203,
            "present-worker",
            TaskClass::Helper,
            1.0,
            1_200_000,
        ),
    ];
    let gpu_samples = vec![GpuSample {
        elapsed_ms: 100,
        gpu_busy_percent: Some(41),
        vram_used_bytes: Some(2_500_000_000),
        vram_total_bytes: Some(8_000_000_000),
        vram_used_percent: Some(31),
        gpu_clock_mhz: Some(1100),
        mem_clock_mhz: Some(6500),
        temp_millidegrees: Some(54_000),
        power_microwatts: Some(62_000_000),
        ..GpuSample::default()
    }];
    let frame_events = vec![
        FrameEvent {
            elapsed_ms: 84,
            frametime_ms: 16.6,
        },
        FrameEvent {
            elapsed_ms: 100,
            frametime_ms: 48.5,
        },
        FrameEvent {
            elapsed_ms: 117,
            frametime_ms: 16.7,
        },
        FrameEvent {
            elapsed_ms: 134,
            frametime_ms: 16.6,
        },
    ];

    let mut session = base_session("compositor_scheduler_delay");
    session.config.tree_roots = vec![5200];
    session.config.hwmon = true;
    session.core.mangohud_first_frame_monotonic_ns = Some(0);
    session.core.mangohud_first_frame_raw_elapsed_ms = Some(0);
    apply_spike_session_fields(&mut session, &spikes);
    apply_artifact_counts(
        &mut session,
        &FixtureArtifacts {
            spikes,
            intervals,
            gpu_samples,
            frame_events,
            ..Default::default()
        },
    )
}

pub(in crate::test_fixture_builder) fn foreground_window_fixture() -> (SessionFile, FixtureArtifacts)
{
    let intervals = vec![interval_record_with_class(
        100,
        5701,
        "Main",
        TaskClass::Game,
        0.0,
        900_000,
    )];
    let foreground_events = vec![ForegroundEvent {
        elapsed_ms: 100,
        source: crate::foreground::ForegroundSource::Sway,
        status: crate::foreground::ForegroundProviderStatus::Available,
        pid: Some(5701),
        app_id: Some("steam_app_sanitized".to_owned()),
        class: Some("steam_app_sanitized".to_owned()),
        title: None,
        window_id: Some("0xSANITIZED".to_owned()),
        workspace: Some("gaming".to_owned()),
        confidence: 0.95,
        stale_ms: None,
        reason: "focused Sway node from sanitized fixture".to_owned(),
    }];

    let mut session = base_session("foreground_window");
    session.config.tree_roots = vec![5701];
    session.config.foreground_window = true;
    session.config.foreground_source = "sway".to_owned();
    session.config.foreground_poll_ms = 1_000;
    session.config.foreground_max_stale_ms = 2_500;
    session.config.foreground_include_title = false;
    session.tasks = vec![task_for_fixture(5701, TaskClass::Game, "Main", 12, 900_000)];
    session.core.active_target_pids_count = 1;
    session.core.active_expanded_tasks = vec![5701];
    session.core.foreground_source = Some("sway".to_owned());
    session.core.final_foreground_pid = Some(5701);
    session.core.final_foreground_app_id = Some("steam_app_sanitized".to_owned());
    session.core.final_foreground_class = Some("steam_app_sanitized".to_owned());
    session.core.final_foreground_status = Some("available".to_owned());
    session.core.final_foreground_window_id = Some("0xSANITIZED".to_owned());
    session.core.final_foreground_workspace = Some("gaming".to_owned());
    session.core.final_foreground_confidence = Some(0.95);
    session.core.final_foreground_stale_ms = None;
    session.core.final_foreground_reason =
        Some("focused Sway node from sanitized fixture".to_owned());

    apply_artifact_counts(
        &mut session,
        &FixtureArtifacts {
            intervals,
            foreground_events,
            ..Default::default()
        },
    )
}

pub(in crate::test_fixture_builder) fn community_rules_classification_fixture()
-> (SessionFile, FixtureArtifacts) {
    let intervals = vec![interval_record_with_class(
        100,
        5801,
        "community-game",
        TaskClass::Game,
        0.0,
        800_000,
    )];

    let mut task = task_for_fixture(5801, TaskClass::Game, "community-game", 12, 800_000);
    task.process_pid = Some(5801);
    task.process_comm = "community-game".into();

    let mut session = base_session("community_rules_classification");
    session.config.tree_roots = vec![5801];
    session.tasks = vec![task];
    session.core.active_target_pids_count = 1;
    session.core.active_expanded_tasks = vec![5801];

    apply_artifact_counts(
        &mut session,
        &FixtureArtifacts {
            intervals,
            ..Default::default()
        },
    )
}
