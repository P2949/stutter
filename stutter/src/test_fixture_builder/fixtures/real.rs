//! Sanitized-real fixture constructors.

use super::{builders::*, *};

pub(in crate::test_fixture_builder) fn real_clean_baseline_fixture()
-> (SessionFile, FixtureArtifacts) {
    renamed_fixture("real_clean_baseline", clean_run_fixture())
}

pub(in crate::test_fixture_builder) fn real_game_thread_scheduler_delay_fixture()
-> (SessionFile, FixtureArtifacts) {
    renamed_fixture(
        "real_game_thread_scheduler_delay",
        game_thread_scheduler_delay_fixture(),
    )
}
pub(in crate::test_fixture_builder) fn real_compositor_scheduler_delay_fixture()
-> (SessionFile, FixtureArtifacts) {
    renamed_fixture(
        "real_compositor_scheduler_delay",
        compositor_scheduler_delay_fixture(),
    )
}

pub(in crate::test_fixture_builder) fn real_irq_overlap_fixture() -> (SessionFile, FixtureArtifacts)
{
    let spikes = vec![
        spike_event(5301, TaskClass::Unknown, "game-worker-a", 6_000_000, 0),
        spike_event(
            5302,
            TaskClass::Unknown,
            "game-worker-b",
            4_500_000,
            250_000,
        ),
        spike_event(
            5303,
            TaskClass::Unknown,
            "game-worker-c",
            4_000_000,
            500_000,
        ),
    ];
    let intervals = vec![
        interval_record_with_class(
            100,
            5301,
            "game-worker-a",
            TaskClass::Unknown,
            4.0,
            6_000_000,
        ),
        interval_record_with_class(
            100,
            5302,
            "game-worker-b",
            TaskClass::Unknown,
            3.0,
            4_500_000,
        ),
        interval_record_with_class(
            100,
            5303,
            "game-worker-c",
            TaskClass::Unknown,
            2.0,
            4_000_000,
        ),
    ];
    let irq_events = vec![
        IrqEventRecord {
            elapsed_ms: Some(42),
            irq: 147,
            cpu: 1,
            enter_ns: 42_000_000,
            exit_ns: 43_000_000,
            duration_ns: 1_000_000,
        },
        IrqEventRecord {
            elapsed_ms: Some(100),
            irq: 146,
            cpu: 3,
            enter_ns: 98_500_000,
            exit_ns: 104_500_000,
            duration_ns: 6_000_000,
        },
        IrqEventRecord {
            elapsed_ms: Some(101),
            irq: 146,
            cpu: 3,
            enter_ns: 100_500_000,
            exit_ns: 103_000_000,
            duration_ns: 2_500_000,
        },
        IrqEventRecord {
            elapsed_ms: Some(178),
            irq: 148,
            cpu: 6,
            enter_ns: 178_000_000,
            exit_ns: 179_250_000,
            duration_ns: 1_250_000,
        },
    ];

    let mut session = base_session("real_irq_overlap");
    session.config.tree_roots = vec![5300];
    session.config.irq_latency = true;
    session.config.irqs = vec![146, 147, 148];
    apply_spike_session_fields(&mut session, &spikes);
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

pub(in crate::test_fixture_builder) fn real_gpu_bound_looking_fixture()
-> (SessionFile, FixtureArtifacts) {
    let spikes = vec![
        spike_event(5501, TaskClass::Unknown, "frame-submit", 2_000_000, 0),
        spike_event(5502, TaskClass::Unknown, "present-wait", 1_800_000, 250_000),
        spike_event(
            5503,
            TaskClass::Unknown,
            "render-helper",
            1_600_000,
            500_000,
        ),
    ];
    let intervals = vec![
        interval_record_with_class(
            100,
            5501,
            "frame-submit",
            TaskClass::Unknown,
            4.0,
            2_000_000,
        ),
        interval_record_with_class(
            100,
            5502,
            "present-wait",
            TaskClass::Unknown,
            3.0,
            1_800_000,
        ),
        interval_record_with_class(
            100,
            5503,
            "render-helper",
            TaskClass::Unknown,
            2.0,
            1_600_000,
        ),
    ];
    let gpu_samples = vec![
        GpuSample {
            elapsed_ms: 84,
            gpu_busy_percent: Some(96),
            vram_used_bytes: Some(7_000_000_000),
            vram_total_bytes: Some(8_000_000_000),
            vram_used_percent: Some(87),
            gpu_clock_mhz: Some(2520),
            mem_clock_mhz: Some(9700),
            temp_millidegrees: Some(70_000),
            power_microwatts: Some(205_000_000),
            ..GpuSample::default()
        },
        GpuSample {
            elapsed_ms: 100,
            gpu_busy_percent: Some(99),
            vram_used_bytes: Some(7_200_000_000),
            vram_total_bytes: Some(8_000_000_000),
            vram_used_percent: Some(90),
            gpu_clock_mhz: Some(2550),
            mem_clock_mhz: Some(9750),
            temp_millidegrees: Some(71_000),
            power_microwatts: Some(215_000_000),
            ..GpuSample::default()
        },
        GpuSample {
            elapsed_ms: 117,
            gpu_busy_percent: Some(98),
            vram_used_bytes: Some(7_250_000_000),
            vram_total_bytes: Some(8_000_000_000),
            vram_used_percent: Some(91),
            gpu_clock_mhz: Some(2530),
            mem_clock_mhz: Some(9750),
            temp_millidegrees: Some(72_000),
            power_microwatts: Some(218_000_000),
            ..GpuSample::default()
        },
    ];
    let frame_events = vec![
        FrameEvent {
            elapsed_ms: 84,
            frametime_ms: 16.6,
        },
        FrameEvent {
            elapsed_ms: 100,
            frametime_ms: 61.0,
        },
        FrameEvent {
            elapsed_ms: 117,
            frametime_ms: 16.8,
        },
        FrameEvent {
            elapsed_ms: 134,
            frametime_ms: 16.7,
        },
    ];

    let mut session = base_session("real_gpu_bound_looking");
    session.config.tree_roots = vec![5500];
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

pub(in crate::test_fixture_builder) fn real_block_io_overlap_fixture()
-> (SessionFile, FixtureArtifacts) {
    let spikes = vec![
        spike_event(5401, TaskClass::Unknown, "asset-stream", 7_000_000, 0),
        spike_event(5402, TaskClass::Unknown, "shader-cache", 4_800_000, 250_000),
        spike_event(5403, TaskClass::Unknown, "io-helper", 4_200_000, 500_000),
    ];
    let intervals = vec![
        interval_record_with_class(
            100,
            5401,
            "asset-stream",
            TaskClass::Unknown,
            2.0,
            7_000_000,
        ),
        interval_record_with_class(
            100,
            5402,
            "shader-cache",
            TaskClass::Unknown,
            1.0,
            4_800_000,
        ),
        interval_record_with_class(100, 5403, "io-helper", TaskClass::Unknown, 1.0, 4_200_000),
    ];
    let block_io_events = vec![
        BlockIoRecord {
            elapsed_ms: 43,
            tid: 5402,
            correlation_basis: Cow::Borrowed("request-pointer"),
            dev: 259,
            nr_sector: 64,
            sector: 4_194_304,
            duration_ns: 2_000_000,
            timestamp_ns: 43_000_000,
            rwbs: "R".to_owned(),
        },
        BlockIoRecord {
            elapsed_ms: 100,
            tid: 5401,
            correlation_basis: Cow::Borrowed("request-pointer"),
            dev: 259,
            nr_sector: 128,
            sector: 8_388_608,
            duration_ns: 12_000_000,
            timestamp_ns: 102_000_000,
            rwbs: "R".to_owned(),
        },
    ];

    let mut session = base_session("real_block_io_overlap");
    session.config.tree_roots = vec![5400];
    session.config.block_io = true;
    session.core.block_io_correlation_basis = "request-pointer".to_owned();
    apply_spike_session_fields(&mut session, &spikes);
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

pub(in crate::test_fixture_builder) fn real_truncated_low_quality_fixture()
-> (SessionFile, FixtureArtifacts) {
    renamed_fixture(
        "real_truncated_low_quality",
        truncated_drop_counters_fixture(),
    )
}

pub(in crate::test_fixture_builder) fn real_foreground_window_fixture()
-> (SessionFile, FixtureArtifacts) {
    let spikes = vec![
        spike_event(5701, TaskClass::Game, "Main", 4_500_000, 0),
        spike_event(
            5702,
            TaskClass::GameHelper,
            "RenderThread",
            3_200_000,
            250_000,
        ),
        spike_event(
            5703,
            TaskClass::Helper,
            "present-helper",
            2_400_000,
            500_000,
        ),
    ];
    let intervals = vec![
        interval_record_with_class(100, 5701, "Main", TaskClass::Game, 2.0, 4_500_000),
        interval_record_with_class(
            100,
            5702,
            "RenderThread",
            TaskClass::GameHelper,
            2.0,
            3_200_000,
        ),
        interval_record_with_class(
            100,
            5703,
            "present-helper",
            TaskClass::Helper,
            1.0,
            2_400_000,
        ),
    ];
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
        confidence: 0.96,
        reason: "focused Sway node from sanitized real foreground fixture".to_owned(),
    }];

    let mut session = base_session("real_foreground_window");
    session.config.tree_roots = vec![5701];
    session.config.foreground_window = true;
    session.config.foreground_source = "sway".to_owned();
    session.config.foreground_poll_ms = 1_000;
    session.config.foreground_max_stale_ms = 2_500;
    session.config.foreground_include_title = false;
    session.core.foreground_source = Some("sway".to_owned());
    session.core.final_foreground_pid = Some(5701);
    session.core.final_foreground_app_id = Some("steam_app_sanitized".to_owned());
    session.core.final_foreground_class = Some("steam_app_sanitized".to_owned());
    apply_spike_session_fields(&mut session, &spikes);
    apply_artifact_counts(
        &mut session,
        &FixtureArtifacts {
            spikes,
            intervals,
            foreground_events,
            ..Default::default()
        },
    )
}

pub(in crate::test_fixture_builder) fn real_community_rules_classification_fixture()
-> (SessionFile, FixtureArtifacts) {
    let spikes = vec![
        spike_event(5801, TaskClass::Game, "community-game", 6_500_000, 0),
        spike_event(
            5802,
            TaskClass::GameWorkerThread,
            "community-worker",
            3_100_000,
            250_000,
        ),
        spike_event(
            5803,
            TaskClass::GameHelper,
            "community-helper",
            2_600_000,
            500_000,
        ),
    ];
    let intervals = vec![
        interval_record_with_class(100, 5801, "community-game", TaskClass::Game, 3.0, 6_500_000),
        interval_record_with_class(
            100,
            5802,
            "community-worker",
            TaskClass::GameWorkerThread,
            2.0,
            3_100_000,
        ),
        interval_record_with_class(
            100,
            5803,
            "community-helper",
            TaskClass::GameHelper,
            1.0,
            2_600_000,
        ),
    ];
    let frame_events = vec![
        FrameEvent {
            elapsed_ms: 84,
            frametime_ms: 16.6,
        },
        FrameEvent {
            elapsed_ms: 100,
            frametime_ms: 46.0,
        },
        FrameEvent {
            elapsed_ms: 117,
            frametime_ms: 16.7,
        },
    ];

    let mut classified_task =
        task_for_fixture(5801, TaskClass::Game, "community-game", 12, 6_500_000);
    classified_task.process_pid = Some(5801);
    classified_task.process_comm = "community-game".into();

    let mut worker_task = task_for_fixture(
        5802,
        TaskClass::GameWorkerThread,
        "community-worker",
        10,
        3_100_000,
    );
    worker_task.process_pid = Some(5801);
    worker_task.process_comm = "community-game".into();

    let mut helper_task = task_for_fixture(
        5803,
        TaskClass::GameHelper,
        "community-helper",
        8,
        2_600_000,
    );
    helper_task.process_pid = Some(5801);
    helper_task.process_comm = "community-game".into();

    let mut session = base_session("real_community_rules_classification");
    session.config.tree_roots = vec![5801];
    session.config.hwmon = false;
    session.core.mangohud_first_frame_monotonic_ns = Some(0);
    session.core.mangohud_first_frame_raw_elapsed_ms = Some(0);
    session.tasks = vec![classified_task, worker_task, helper_task];
    session.core.active_target_pids_count = 1;
    session.core.active_expanded_tasks = vec![5801, 5802, 5803];
    apply_spike_session_fields(&mut session, &spikes);
    apply_artifact_counts(
        &mut session,
        &FixtureArtifacts {
            spikes,
            intervals,
            frame_events,
            ..Default::default()
        },
    )
}
