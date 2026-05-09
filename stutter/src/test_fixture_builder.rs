use std::{
    borrow::Cow,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::Context;

use crate::{
    ebpf_loader::DropCountersSnapshot,
    metadata::SystemMetadata,
    process_tree::TaskClass,
    recorder::{
        BlockIoRecord, ForegroundEvent, FrameEvent, GpuSample, IntervalRecord, IrqEventRecord,
        MetadataFile, RecordedConfig, RecordedCpuSnapshot, RecordedLatency, RecordedTime,
        SESSION_SCHEMA_VERSION, SessionFile, SessionTask, SpikeEvent,
    },
};

const OPTIONAL_ARTIFACT_FILES: &[&str] = &[
    "spike_events.json",
    "interval.json",
    "tree_events.json",
    "irq_events.json",
    "gpu_samples.json",
    "frame_correlation.json",
    "frame_events.json",
    "migration_events.json",
    "cpu_freq_samples.json",
    "io_events.json",
    "scx_events.json",
    "focus_events.json",
    "foreground_events.json",
];

#[derive(serde::Serialize)]
struct FixtureMetadata {
    name: String,
    schema_version: u32,
    source: String,
    quality_expectation: String,
    description: String,
    expected: FixtureExpected,
    privacy: FixturePrivacy,
}

#[derive(serde::Serialize)]
struct FixtureExpected {
    primary_cause: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    required_candidate: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    required_candidate_evidence: Vec<String>,
    accepted_confidence: Vec<String>,
    data_quality: String,
    artifacts: FixtureExpectedArtifacts,
    evidence: FixtureExpectedEvidence,
}

#[derive(Default, serde::Serialize)]
struct FixtureExpectedArtifacts {
    #[serde(skip_serializing_if = "Option::is_none")]
    spikes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    spikes_min: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    intervals: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    intervals_min: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    irq_events: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    irq_events_min: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gpu_samples: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gpu_samples_min: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frames: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frames_min: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    block_io_events: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    block_io_events_min: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    foreground_events: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    foreground_events_min: Option<u64>,
}

#[derive(serde::Serialize)]
struct FixtureExpectedEvidence {
    contains: Vec<String>,
}

#[derive(serde::Serialize)]
struct FixturePrivacy {
    titles_redacted: bool,
    paths_redacted: bool,
    hostnames_redacted: bool,
    usernames_redacted: bool,
}

#[derive(Default)]
pub(crate) struct FixtureArtifacts {
    pub(crate) spikes: Vec<SpikeEvent>,
    pub(crate) intervals: Vec<IntervalRecord>,
    pub(crate) irq_events: Vec<IrqEventRecord>,
    pub(crate) gpu_samples: Vec<GpuSample>,
    pub(crate) frame_events: Vec<FrameEvent>,
    pub(crate) block_io_events: Vec<BlockIoRecord>,
    pub(crate) foreground_events: Vec<ForegroundEvent>,
}

pub(crate) fn write_validation_corpus(root: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(root)
        .with_context(|| format!("failed to create fixture root {}", root.display()))?;

    for deprecated in [
        "real_world_game_scheduler_delay",
        "real_world_compositor_scheduler_delay",
        "real_world_irq_overlap",
        "real_world_block_io_stall",
        "real_world_gpu_bound_clean_cpu",
        "real_block_io_overlap",
        "real_truncated_low_quality",
        "real_foreground_window",
        "real_community_rules_classification",
    ] {
        remove_fixture_dir(root, deprecated)?;
    }

    write_fixture(root, "clean_run", clean_run_fixture())?;
    write_fixture(root, "cpu_pressure", cpu_pressure_fixture())?;
    write_fixture(root, "block_io_stall", block_io_stall_fixture())?;
    write_fixture(root, "irq_heavy", irq_heavy_fixture())?;
    write_fixture(root, "gpu_bound_clean_cpu", gpu_bound_clean_cpu_fixture())?;
    write_fixture(
        root,
        "truncated_drop_counters",
        truncated_drop_counters_fixture(),
    )?;
    write_fixture(
        root,
        "reused_tid_no_contamination",
        reused_tid_no_contamination_fixture(),
    )?;
    write_fixture(root, "old_schema_warning", old_schema_warning_fixture())?;

    write_fixture(
        root,
        "game_thread_scheduler_delay",
        game_thread_scheduler_delay_fixture(),
    )?;
    write_fixture(
        root,
        "compositor_scheduler_delay",
        compositor_scheduler_delay_fixture(),
    )?;
    write_fixture(root, "foreground_window", foreground_window_fixture())?;
    write_fixture(
        root,
        "community_rules_classification",
        community_rules_classification_fixture(),
    )?;
    write_fixture(root, "real_clean_baseline", real_clean_baseline_fixture())?;
    write_fixture(
        root,
        "real_game_thread_scheduler_delay",
        real_game_thread_scheduler_delay_fixture(),
    )?;
    write_fixture(
        root,
        "real_compositor_scheduler_delay",
        real_compositor_scheduler_delay_fixture(),
    )?;
    write_fixture(root, "real_irq_overlap", real_irq_overlap_fixture())?;
    write_fixture(
        root,
        "real_gpu_bound_looking",
        real_gpu_bound_looking_fixture(),
    )?;

    Ok(())
}

pub(crate) fn write_public_examples_v21(root: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(root)
        .with_context(|| format!("failed to create public example root {}", root.display()))?;

    for deprecated in [
        "real_world_game_scheduler_delay",
        "real_world_compositor_scheduler_delay",
        "real_world_gpu_bound_clean_cpu",
    ] {
        remove_fixture_dir(root, deprecated)?;
    }

    write_fixture(root, "clean_baseline", public_clean_baseline_fixture())?;
    write_fixture(
        root,
        "game_thread_scheduler_delay",
        renamed_fixture(
            "game_thread_scheduler_delay",
            public_game_thread_scheduler_delay_fixture(),
        ),
    )?;
    write_fixture(
        root,
        "low_quality_truncated",
        public_low_quality_truncated_fixture(),
    )?;
    write_public_examples_readme_v21(root)?;

    Ok(())
}

pub(crate) fn write_autotune_replay_corpus(root: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(root)
        .with_context(|| format!("failed to create replay fixture root {}", root.display()))?;

    write_fixture(
        root,
        "game_scheduler_pressure",
        game_scheduler_pressure_fixture(),
    )?;
    write_fixture(root, "gpu_bound", gpu_bound_clean_cpu_fixture())?;
    write_fixture(root, "low_quality", truncated_drop_counters_fixture())?;

    Ok(())
}

fn remove_fixture_dir(root: &Path, name: &str) -> anyhow::Result<()> {
    let dir = root.join(name);
    if dir.exists() {
        fs::remove_dir_all(&dir).with_context(|| {
            format!("failed to remove deprecated fixture dir {}", dir.display())
        })?;
    }
    Ok(())
}

fn renamed_fixture(
    name: &str,
    (mut session, artifacts): (SessionFile, FixtureArtifacts),
) -> (SessionFile, FixtureArtifacts) {
    session.core.run_name = Some(name.to_owned());
    (session, artifacts)
}

fn public_clean_baseline_fixture() -> (SessionFile, FixtureArtifacts) {
    renamed_fixture("clean_baseline", clean_run_fixture())
}

fn public_game_thread_scheduler_delay_fixture() -> (SessionFile, FixtureArtifacts) {
    renamed_fixture(
        "game_thread_scheduler_delay_public",
        game_thread_scheduler_delay_fixture(),
    )
}

fn real_clean_baseline_fixture() -> (SessionFile, FixtureArtifacts) {
    renamed_fixture("real_clean_baseline", clean_run_fixture())
}

fn real_game_thread_scheduler_delay_fixture() -> (SessionFile, FixtureArtifacts) {
    renamed_fixture(
        "real_game_thread_scheduler_delay",
        game_thread_scheduler_delay_fixture(),
    )
}
fn real_compositor_scheduler_delay_fixture() -> (SessionFile, FixtureArtifacts) {
    renamed_fixture(
        "real_compositor_scheduler_delay",
        compositor_scheduler_delay_fixture(),
    )
}

fn real_irq_overlap_fixture() -> (SessionFile, FixtureArtifacts) {
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

fn real_gpu_bound_looking_fixture() -> (SessionFile, FixtureArtifacts) {
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

fn public_low_quality_truncated_fixture() -> (SessionFile, FixtureArtifacts) {
    renamed_fixture("low_quality_truncated", truncated_drop_counters_fixture())
}

pub(crate) fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("runs")
        .join(name)
}

fn game_scheduler_pressure_fixture() -> (SessionFile, FixtureArtifacts) {
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

fn cpu_pressure_fixture() -> (SessionFile, FixtureArtifacts) {
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

fn block_io_stall_fixture() -> (SessionFile, FixtureArtifacts) {
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

fn irq_heavy_fixture() -> (SessionFile, FixtureArtifacts) {
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

fn gpu_bound_clean_cpu_fixture() -> (SessionFile, FixtureArtifacts) {
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

fn clean_run_fixture() -> (SessionFile, FixtureArtifacts) {
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

fn truncated_drop_counters_fixture() -> (SessionFile, FixtureArtifacts) {
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
        ringbuf_reserve_failed: 1,
        irq_start_times_insert_failed: 0,
        block_start_insert_failed: 0,
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

fn reused_tid_no_contamination_fixture() -> (SessionFile, FixtureArtifacts) {
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

fn old_schema_warning_fixture() -> (SessionFile, FixtureArtifacts) {
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

fn game_thread_scheduler_delay_fixture() -> (SessionFile, FixtureArtifacts) {
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

fn compositor_scheduler_delay_fixture() -> (SessionFile, FixtureArtifacts) {
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

fn foreground_window_fixture() -> (SessionFile, FixtureArtifacts) {
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

    apply_artifact_counts(
        &mut session,
        &FixtureArtifacts {
            intervals,
            foreground_events,
            ..Default::default()
        },
    )
}

fn community_rules_classification_fixture() -> (SessionFile, FixtureArtifacts) {
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

fn apply_artifact_counts(
    session: &mut SessionFile,
    artifacts: &FixtureArtifacts,
) -> (SessionFile, FixtureArtifacts) {
    session.core.spike_events_retained_count = artifacts.spikes.len() as u64;
    session.core.interval_record_count = artifacts.intervals.len() as u64;
    session.core.irq_event_count = artifacts.irq_events.len() as u64;
    session.core.gpu_sample_count = artifacts.gpu_samples.len() as u64;
    session.core.frame_event_count = artifacts.frame_events.len() as u64;
    session.core.block_io_event_count = artifacts.block_io_events.len() as u64;
    session.core.foreground_event_count = artifacts.foreground_events.len() as u64;
    (
        session.clone(),
        FixtureArtifacts {
            spikes: artifacts.spikes.clone(),
            intervals: artifacts.intervals.clone(),
            irq_events: artifacts.irq_events.clone(),
            gpu_samples: artifacts.gpu_samples.clone(),
            frame_events: artifacts.frame_events.clone(),
            block_io_events: artifacts.block_io_events.clone(),
            foreground_events: artifacts.foreground_events.clone(),
        },
    )
}

fn write_fixture(
    root: &Path,
    name: &str,
    (session, artifacts): (SessionFile, FixtureArtifacts),
) -> anyhow::Result<()> {
    let dir = root.join(name);
    if dir.exists() {
        fs::remove_dir_all(&dir)
            .with_context(|| format!("failed to remove fixture dir {}", dir.display()))?;
    }
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create fixture dir {}", dir.display()))?;

    let fixture_metadata = fixture_metadata_for(name, &artifacts);

    write_toml_pretty(dir.join("fixture.toml"), &fixture_metadata)?;
    write_json_pretty(dir.join("session.json"), &session)?;
    write_json_pretty(
        dir.join("metadata.json"),
        &MetadataFile {
            core: session.core.clone(),
        },
    )?;

    for file in OPTIONAL_ARTIFACT_FILES {
        write_ndjson_values::<serde_json::Value>(dir.join(file), &[])?;
    }

    write_ndjson_values(dir.join("spike_events.json"), &artifacts.spikes)?;
    write_ndjson_values(dir.join("interval.json"), &artifacts.intervals)?;
    write_ndjson_values(dir.join("irq_events.json"), &artifacts.irq_events)?;
    write_ndjson_values(dir.join("gpu_samples.json"), &artifacts.gpu_samples)?;
    write_ndjson_values(dir.join("frame_correlation.json"), &artifacts.frame_events)?;
    write_ndjson_values(dir.join("io_events.json"), &artifacts.block_io_events)?;
    write_ndjson_values(
        dir.join("foreground_events.json"),
        &artifacts.foreground_events,
    )?;

    Ok(())
}

fn fixture_metadata_for(name: &str, artifacts: &FixtureArtifacts) -> FixtureMetadata {
    match name {
        "clean_run" => fixture_metadata(
            name,
            "synthetic-contract",
            "High",
            "Synthetic clean run fixture that should remain high quality and produce no strong diagnosis.",
            "Unknown",
            &[],
            "High",
            &[],
            exact_artifacts(artifacts),
        ),
        "cpu_pressure" => fixture_metadata(
            name,
            "synthetic-contract",
            "High",
            "Synthetic CPU pressure fixture with high CPU PSI near a scheduler-latency spike.",
            "CpuPressureCandidate",
            &["Medium", "High"],
            "High",
            &["high CPU PSI"],
            exact_artifacts(artifacts),
        ),
        "block_io_stall" => fixture_metadata(
            name,
            "synthetic-contract",
            "High",
            "Synthetic block I/O fixture with a long request overlapping a scheduler-latency spike.",
            "BlockIoCandidate",
            &["Medium", "High"],
            "High",
            &["block I/O"],
            exact_artifacts(artifacts),
        ),
        "irq_heavy" => fixture_metadata(
            name,
            "synthetic-contract",
            "High",
            "Synthetic IRQ fixture with a long IRQ handler overlapping scheduler-latency spikes.",
            "IrqDelayCandidate",
            &["Medium", "High"],
            "High",
            &["IRQ"],
            exact_artifacts(artifacts),
        ),
        "gpu_bound_clean_cpu" => fixture_metadata(
            name,
            "synthetic-contract",
            "High",
            "Synthetic GPU-bound fixture with high GPU busy and clean CPU pressure.",
            "GpuBoundCandidate",
            &["Low", "Medium", "High"],
            "High",
            &["GPU busy"],
            exact_artifacts(artifacts),
        ),
        "truncated_drop_counters" => fixture_metadata(
            name,
            "synthetic-contract",
            "Medium",
            "Synthetic low-quality fixture with truncated spike events and non-zero drop counters.",
            "Unknown",
            &[],
            "Medium",
            &[],
            exact_artifacts(artifacts),
        ),
        "reused_tid_no_contamination" => fixture_metadata(
            name,
            "synthetic-contract",
            "High",
            "Synthetic reused-TID fixture that verifies separate logical tasks are not merged.",
            "Unknown",
            &[],
            "High",
            &[],
            exact_artifacts(artifacts),
        ),
        "old_schema_warning" => fixture_metadata(
            name,
            "synthetic-contract",
            "Medium",
            "Synthetic old-schema fixture that should warn without being rejected.",
            "Unknown",
            &[],
            "Medium",
            &[],
            exact_artifacts(artifacts),
        ),
        "game_thread_scheduler_delay" => fixture_metadata(
            name,
            "synthetic-edge-case",
            "High",
            "Synthetic edge-case fixture for game main/render thread scheduler delay during a visible frame spike.",
            "GameThreadSchedulerDelay",
            &["Medium", "High"],
            "High",
            &["game thread", "delayed"],
            exact_artifacts(artifacts),
        ),
        "compositor_scheduler_delay" => fixture_metadata(
            name,
            "synthetic-edge-case",
            "High",
            "Synthetic edge-case fixture for compositor thread scheduler delay during a visible frame spike.",
            "CompositorSchedulerDelay",
            &["Medium", "High"],
            "High",
            &["compositor thread", "delayed"],
            exact_artifacts(artifacts),
        ),
        "real_gpu_bound_looking" => {
            let mut metadata = fixture_metadata(
                name,
                "sanitized-real-recording",
                "High",
                "GPU busy was high during a visible frame spike; scheduler evidence may also exist, so GPU-bound is required as a candidate rather than always primary.",
                "Any",
                &[],
                "High",
                &[],
                exact_artifacts(artifacts),
            );
            metadata.expected.required_candidate = Some("GpuBoundCandidate".to_owned());
            metadata.expected.required_candidate_evidence = vec!["GPU busy".to_owned()];
            metadata
        }
        "foreground_window" => fixture_metadata(
            name,
            "synthetic-edge-case",
            "High",
            "Synthetic edge-case fixture that verifies foreground PID/app/class are preserved while title is redacted.",
            "Unknown",
            &[],
            "High",
            &[],
            exact_artifacts(artifacts),
        ),
        "community_rules_classification" => fixture_metadata(
            name,
            "synthetic-edge-case",
            "High",
            "Synthetic edge-case fixture that verifies a community-classified game task remains classified as Game.",
            "Unknown",
            &[],
            "High",
            &[],
            exact_artifacts(artifacts),
        ),
        "clean_baseline" => fixture_metadata(
            name,
            "public-example",
            "High",
            "Small public clean baseline example.",
            "Unknown",
            &[],
            "High",
            &[],
            exact_artifacts(artifacts),
        ),
        "game_thread_scheduler_delay_public" => fixture_metadata(
            name,
            "public-example",
            "High",
            "Small public game-thread scheduler delay example.",
            "GameThreadSchedulerDelay",
            &["Medium", "High"],
            "High",
            &["game thread", "delayed"],
            exact_artifacts(artifacts),
        ),
        "low_quality_truncated" => fixture_metadata(
            name,
            "public-example",
            "Medium",
            "Small public low-quality truncated example.",
            "Unknown",
            &[],
            "Medium",
            &[],
            exact_artifacts(artifacts),
        ),
        "game_scheduler_pressure" => fixture_metadata(
            name,
            "autotune-replay",
            "High",
            "Autotune replay fixture with game scheduler pressure.",
            "GameThreadSchedulerDelay",
            &["Medium", "High"],
            "High",
            &["game thread"],
            exact_artifacts(artifacts),
        ),
        "gpu_bound" => fixture_metadata(
            name,
            "autotune-replay",
            "High",
            "Autotune replay fixture for a GPU-bound run.",
            "GpuBoundCandidate",
            &["Low", "Medium", "High"],
            "High",
            &["GPU busy"],
            exact_artifacts(artifacts),
        ),
        "low_quality" => fixture_metadata(
            name,
            "autotune-replay",
            "Medium",
            "Autotune replay fixture for a low-quality run with dropped or truncated data.",
            "Unknown",
            &[],
            "Medium",
            &[],
            exact_artifacts(artifacts),
        ),
        "real_clean_baseline" => fixture_metadata(
            name,
            "validation-corpus",
            "High",
            "Real clean baseline example.",
            "Unknown",
            &[],
            "High",
            &[],
            exact_artifacts(artifacts),
        ),
        "real_compositor_scheduler_delay" => fixture_metadata(
            name,
            "sanitized-real-recording",
            "High",
            "Compositor or gamescope thread had scheduler delay during a visible frame spike.",
            "CompositorSchedulerDelay",
            &["Medium", "High"],
            "High",
            &["compositor thread"],
            exact_artifacts(artifacts),
        ),
        "real_game_thread_scheduler_delay" => fixture_metadata(
            name,
            "validation-corpus",
            "High",
            "Real game-thread scheduler delay example.",
            "GameThreadSchedulerDelay",
            &["Medium", "High"],
            "High",
            &["game thread", "delayed"],
            exact_artifacts(artifacts),
        ),
        "real_irq_overlap" => fixture_metadata(
            name,
            "sanitized-real-recording",
            "High",
            "IRQ handler activity overlapped the scheduler-latency cluster while unrelated IRQ noise occurred outside the correlation window.",
            "IrqDelayCandidate",
            &["Medium", "High"],
            "High",
            &["IRQ"],
            exact_artifacts(artifacts),
        ),
        other => fixture_metadata(
            other,
            "synthetic-contract",
            "High",
            "Generated synthetic fixture.",
            "Unknown",
            &[],
            "High",
            &[],
            exact_artifacts(artifacts),
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn fixture_metadata(
    name: &str,
    source: &str,
    quality_expectation: &str,
    description: &str,
    primary_cause: &str,
    accepted_confidence: &[&str],
    data_quality: &str,
    evidence_contains: &[&str],
    artifacts: FixtureExpectedArtifacts,
) -> FixtureMetadata {
    FixtureMetadata {
        name: name.to_owned(),
        schema_version: SESSION_SCHEMA_VERSION,
        source: source.to_owned(),
        quality_expectation: quality_expectation.to_owned(),
        description: description.to_owned(),
        expected: FixtureExpected {
            primary_cause: primary_cause.to_owned(),
            required_candidate: None,
            required_candidate_evidence: Vec::new(),
            accepted_confidence: accepted_confidence
                .iter()
                .map(|item| (*item).to_owned())
                .collect(),
            data_quality: data_quality.to_owned(),
            artifacts,
            evidence: FixtureExpectedEvidence {
                contains: evidence_contains
                    .iter()
                    .map(|item| (*item).to_owned())
                    .collect(),
            },
        },
        privacy: FixturePrivacy {
            titles_redacted: true,
            paths_redacted: true,
            hostnames_redacted: true,
            usernames_redacted: true,
        },
    }
}

fn exact_artifacts(artifacts: &FixtureArtifacts) -> FixtureExpectedArtifacts {
    FixtureExpectedArtifacts {
        spikes: Some(artifacts.spikes.len() as u64),
        spikes_min: None,
        intervals: Some(artifacts.intervals.len() as u64),
        intervals_min: None,
        irq_events: Some(artifacts.irq_events.len() as u64),
        irq_events_min: None,
        gpu_samples: Some(artifacts.gpu_samples.len() as u64),
        gpu_samples_min: None,
        frames: Some(artifacts.frame_events.len() as u64),
        frames_min: None,
        block_io_events: Some(artifacts.block_io_events.len() as u64),
        block_io_events_min: None,
        foreground_events: Some(artifacts.foreground_events.len() as u64),
        foreground_events_min: None,
    }
}

fn write_public_examples_readme_v21(root: &Path) -> anyhow::Result<()> {
    let readme = r#"# stutter v21 public artifact examples

This directory intentionally contains only small, representative sanitized examples.

## Examples

| Directory                      | Purpose                                        |
| ------------------------------ | ---------------------------------------------- |
| `clean_baseline/`              | Quiet baseline run with no strong diagnosis.   |
| `game_thread_scheduler_delay/` | Game-thread scheduler-delay diagnosis example. |
| `low_quality_truncated/`       | Low-quality/truncated data-quality example.    |

The larger regression corpus lives under:

```text
stutter/tests/fixtures/runs/
```

Do not duplicate every large validation fixture here unless repository size stays reasonable.
"#;

    fs::write(root.join("README.md"), readme).with_context(|| {
        format!(
            "failed to write public examples README under {}",
            root.display()
        )
    })
}

fn write_toml_pretty<T: serde::Serialize>(path: impl AsRef<Path>, value: &T) -> anyhow::Result<()> {
    let path = path.as_ref();
    let text = toml::to_string_pretty(value)
        .with_context(|| format!("failed to serialize TOML fixture {}", path.display()))?;
    fs::write(path, text)
        .with_context(|| format!("failed to write TOML fixture {}", path.display()))?;
    Ok(())
}

fn write_json_pretty<T: serde::Serialize>(path: impl AsRef<Path>, value: &T) -> anyhow::Result<()> {
    let path = path.as_ref();
    let file = fs::File::create(path)
        .with_context(|| format!("failed to create JSON fixture {}", path.display()))?;
    serde_json::to_writer_pretty(file, value)
        .with_context(|| format!("failed to write JSON fixture {}", path.display()))?;
    Ok(())
}

fn write_ndjson_values<T: serde::Serialize>(
    path: impl AsRef<Path>,
    values: &[T],
) -> anyhow::Result<()> {
    let path = path.as_ref();
    let mut file = fs::File::create(path)
        .with_context(|| format!("failed to create NDJSON fixture {}", path.display()))?;
    for value in values {
        serde_json::to_writer(&mut file, value)
            .with_context(|| format!("failed to write NDJSON fixture {}", path.display()))?;
        file.write_all(b"\n")
            .with_context(|| format!("failed to write NDJSON fixture {}", path.display()))?;
    }
    Ok(())
}

fn unknown_clustered_spikes(anchor_latency_ns: u64) -> Vec<SpikeEvent> {
    vec![
        spike_event(100, TaskClass::Unknown, "worker-a", anchor_latency_ns, 0),
        spike_event(101, TaskClass::Unknown, "worker-b", 2_500_000, 250_000),
        spike_event(102, TaskClass::Unknown, "worker-c", 2_000_000, 500_000),
    ]
}

fn spike_event(
    task: u32,
    class: TaskClass,
    comm: &str,
    latency_ns: u64,
    offset_ns: u64,
) -> SpikeEvent {
    let switch_ns = 100_000_000 + offset_ns;
    SpikeEvent {
        elapsed_ms: Some(100),
        task,
        active: true,
        class,
        process_pid: Some(task),
        process_comm: comm.into(),
        comm: comm.to_owned(),
        cpu: 0,
        wakeup_target_cpu: 0,
        prio: 120,
        latency_ns,
        wakeup_ns: switch_ns.saturating_sub(latency_ns),
        switch_ns,
        switch_prev_pid: 0,
        switch_prev_state: 0,
        switch_prev_state_label: "running".to_owned(),
        ..Default::default()
    }
}

fn interval_record(elapsed_ms: u64, task: u32, comm: &str, cpu_psi_some: f64) -> IntervalRecord {
    IntervalRecord {
        elapsed_ms,
        task,
        active: true,
        class: TaskClass::Unknown,
        comm: comm.to_owned(),
        process_pid: Some(task),
        process_comm: comm.into(),
        samples: 10,
        stored_samples: 10,
        truncated_samples: 0,
        min_ns: 100,
        avg_ns: 500,
        p95_ns: 1_000,
        p99_ns: 2_000,
        max_ns: 5_000,
        over_1ms: 0,
        over_2ms: 0,
        over_5ms: 0,
        busiest_cpu: Some(0),
        busiest_cpu_samples: 10,
        worst_cpu: Some(0),
        worst_cpu_max_ns: 5_000,
        spikiest_cpu: Some(0),
        spikiest_cpu_spikes: 0,
        major_faults: 0,
        minor_faults: 0,
        cpu_psi_some,
        mem_psi_some: 0.0,
        mem_psi_full: 0.0,
        io_psi_some: 0.0,
        io_psi_full: 0.0,
        percentile_scope: "exact".to_owned(),
        histogram: vec![],
        drop_counters: DropCountersSnapshot::default(),
        cpu_perf: None,
    }
}

fn interval_record_with_class(
    elapsed_ms: u64,
    task: u32,
    comm: &str,
    class: TaskClass,
    cpu_psi_some: f64,
    max_ns: u64,
) -> IntervalRecord {
    IntervalRecord {
        elapsed_ms,
        task,
        active: true,
        class,
        comm: comm.to_owned(),
        process_pid: Some(task),
        process_comm: comm.into(),
        samples: 10,
        stored_samples: 10,
        truncated_samples: 0,
        min_ns: 100,
        avg_ns: max_ns / 4,
        p95_ns: max_ns.saturating_sub(250_000),
        p99_ns: max_ns.saturating_sub(1),
        max_ns,
        over_1ms: u64::from(max_ns > 1_000_000),
        over_2ms: u64::from(max_ns > 2_000_000),
        over_5ms: u64::from(max_ns > 5_000_000),
        busiest_cpu: Some(0),
        busiest_cpu_samples: 10,
        worst_cpu: Some(0),
        worst_cpu_max_ns: max_ns,
        spikiest_cpu: Some(0),
        spikiest_cpu_spikes: u64::from(max_ns > 1_000_000),
        major_faults: 0,
        minor_faults: 0,
        cpu_psi_some,
        mem_psi_some: 0.0,
        mem_psi_full: 0.0,
        io_psi_some: 0.0,
        io_psi_full: 0.0,
        percentile_scope: "exact".to_owned(),
        histogram: vec![],
        drop_counters: DropCountersSnapshot::default(),
        cpu_perf: None,
    }
}

fn apply_spike_session_fields(session: &mut SessionFile, spikes: &[SpikeEvent]) {
    session.core.active_target_pids_count = spikes.len() as u64;
    session.core.active_expanded_tasks = spikes.iter().map(|spike| spike.task).collect();
    session.tasks = spikes
        .iter()
        .map(|spike| task_for_fixture(spike.task, spike.class, &spike.comm, 10, spike.latency_ns))
        .collect();
}

fn task_for_fixture(
    task: u32,
    class: TaskClass,
    comm: &str,
    samples: u64,
    max_latency_ns: u64,
) -> SessionTask {
    let over_1ms = u64::from(max_latency_ns > 1_000_000);
    let over_2ms = u64::from(max_latency_ns > 2_000_000);
    let over_5ms = u64::from(max_latency_ns > 5_000_000);

    SessionTask {
        task,
        active: true,
        first_seen_ms: 0,
        last_seen_ms: 1000,
        removed_ms: None,
        class,
        process_pid: Some(task),
        process_comm: comm.into(),
        process_starttime_ticks: Some(1_000 + u64::from(task)),
        task_starttime_ticks: Some(2_000 + u64::from(task)),
        exe_dev: Some(10),
        exe_ino: Some(10_000 + u64::from(task)),
        comm: comm.to_owned(),
        latency: RecordedLatency {
            samples,
            stored_samples: samples,
            truncated_samples: 0,
            percentile_scope: "exact".to_owned(),
            histogram: vec![],
            min_ns: 100,
            avg_ns: 500,
            p95_ns: max_latency_ns / 2,
            p99_ns: max_latency_ns.saturating_sub(1),
            max_ns: max_latency_ns,
            over_1ms,
            over_2ms,
            over_5ms,
        },
        cpu: RecordedCpuSnapshot {
            busiest_cpu: Some(0),
            busiest_cpu_samples: samples,
            worst_cpu: Some(0),
            worst_cpu_max_ns: max_latency_ns,
            spikiest_cpu: Some(0),
            spikiest_cpu_spikes: over_1ms,
            per_cpu: vec![],
        },
        top_spikes: vec![],
        migration_count: 0,
        cross_numa_migrations: 0,
        top_wakers: vec![],
        sched_policy: None,
        stat_wait_sum_ns: None,
        stat_wait_sum_ns_saturated: false,
        stat_wait_count: None,
        cpu_perf: None,
    }
}

fn base_session(run_name: &str) -> SessionFile {
    SessionFile {
        core: crate::recorder::SessionMetadataCore {
            schema_version: SESSION_SCHEMA_VERSION,
            run_name: Some(run_name.to_owned()),
            started_at: dummy_time(),
            ended_at: dummy_time(),
            monotonic_start_ns: Some(0),
            monotonic_end_ns: Some(1_000_000_000),
            duration_ms: 1000,
            mangohud_start_offset: None,
            mangohud_first_frame_monotonic_ns: None,
            mangohud_first_frame_raw_elapsed_ms: None,
            metadata: SystemMetadata::default(),
            target_pids_max: 1024,
            active_target_pids_count: 1,
            active_expanded_tasks: vec![100],
            interval_record_count: 1,
            intervals_dropped: 0,
            spike_events_retained_count: 0,
            spike_events_dropped_count: 0,
            spike_events_truncated: false,
            scx_event_count: 0,
            irq_event_count: 0,
            migration_event_count: Some(0),
            cpu_freq_sample_count: Some(0),
            gpu_sample_count: 0,
            frame_event_count: 0,
            block_io_event_count: 0,
            event_stream_write_errors: 0,
            alert_events_dropped_count: 0,
            alert_channel_closed_count: 0,
            first_event_stream_write_error: None,
            block_io_correlation_basis: "dev+sector".to_owned(),
            drop_counters: DropCountersSnapshot::default(),
            cpu_perf_sample_count: 0,
            cpu_perf_open_errors: 0,
            cpu_perf_read_errors: 0,
            cpu_perf_skipped_tasks: 0,
            cpu_perf_last_error: None,
            ..Default::default()
        },
        stop_reason: "test".to_owned(),
        config: dummy_config(),
        tasks: vec![task_for_fixture(
            100,
            TaskClass::Unknown,
            "worker-a",
            10,
            5_000,
        )],
        top_spikes: vec![],
    }
}

fn dummy_time() -> RecordedTime {
    RecordedTime {
        unix_seconds: 1_625_097_600,
        unix_nanos: 0,
        system_time_debug: "2021-07-01T00:00:00Z".to_owned(),
    }
}

fn dummy_config() -> RecordedConfig {
    RecordedConfig {
        manual_pids: vec![],
        tree_roots: vec![],
        cgroupv2: None,
        exclude_tree_pids: vec![],
        include_comm: vec![],
        exclude_comm: vec![],
        watch_process: None,
        persistent: false,
        keep_missing_pid: false,
        watch_poll_ms: 100,
        watch_timeout_ms: None,
        csv_stream: None,
        irq_latency: false,
        irqs: vec![],
        hwmon: false,
        hwmon_root: None,
        hwmon_drm_card: None,
        hwmon_render_node: None,
        mangohud_log: None,
        mangohud_log_live: false,
        tui: false,
        summary_period_ms: 1000,
        epoch_period_ms: None,
        retain_intervals: None,
        max_tasks: 1024,
        spike_threshold_ns: 1_000_000,
        alert_threshold_ns: None,
        alert_webhook_url: None,
        follow_exec: true,
        verbose: false,
        faults: false,
        cpu_perf: false,
        cpu_perf_kernel: false,
        cpu_perf_max_tasks: 128,
        cpu_perf_cache_refs: false,
        block_io: false,
        stat_wait: false,
        otlp_endpoint: None,
        otel_service_name: "stutter".to_owned(),
        ..Default::default()
    }
}
