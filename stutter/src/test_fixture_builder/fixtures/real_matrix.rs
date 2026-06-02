//! Sanitized-real matrix fixture constructors.
//!
//! These are minimized, redacted recordings derived from real capture shapes.
//! Keep names, process comms, paths, titles, hostnames, usernames, and window IDs sanitized.

use super::{builders::*, *};

pub(in crate::test_fixture_builder) fn real_amd_hyprland_clean_fixture()
-> (SessionFile, FixtureArtifacts) {
    let mut session = platform_session("real_amd_hyprland_clean", "AMD", "amdgpu", "Hyprland");
    session.config.tree_roots = vec![6100];
    session.config.hwmon = true;
    session.tasks = vec![task_for_fixture(
        6101,
        TaskClass::Game,
        "game-main",
        20,
        600_000,
    )];
    session.core.active_target_pids_count = 1;
    session.core.active_expanded_tasks = vec![6101];
    session.core.mangohud_first_frame_monotonic_ns = Some(0);
    session.core.mangohud_first_frame_raw_elapsed_ms = Some(0);

    let intervals = vec![
        interval_record_with_class(100, 6101, "game-main", TaskClass::Game, 0.0, 600_000),
        interval_record_with_class(200, 6101, "game-main", TaskClass::Game, 0.0, 650_000),
    ];
    let gpu_samples = vec![
        gpu_sample(100, "card0", "renderD128", 34, 1420, 75_000_000),
        gpu_sample(200, "card0", "renderD128", 38, 1440, 78_000_000),
    ];
    let frame_events = vec![
        FrameEvent {
            elapsed_ms: 100,
            frametime_ms: 16.6,
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

    apply_artifact_counts(
        &mut session,
        &FixtureArtifacts {
            intervals,
            gpu_samples,
            frame_events,
            display_topology: Some(display_topology_snapshot("AMD", "amdgpu", "Hyprland")),
            ..Default::default()
        },
    )
}

pub(in crate::test_fixture_builder) fn real_nvidia_gnome_false_positive_fixture()
-> (SessionFile, FixtureArtifacts) {
    let mut session = platform_session(
        "real_nvidia_gnome_false_positive",
        "NVIDIA",
        "nvidia",
        "GNOME",
    );
    session.config.tree_roots = vec![6200];
    session.config.hwmon = true;
    session.tasks = vec![
        task_for_fixture(6201, TaskClass::Game, "game-main", 20, 700_000),
        task_for_fixture(6202, TaskClass::Compositor, "gnome-shell", 20, 800_000),
    ];
    session.core.active_target_pids_count = 2;
    session.core.active_expanded_tasks = vec![6201, 6202];
    session.core.mangohud_first_frame_monotonic_ns = Some(0);
    session.core.mangohud_first_frame_raw_elapsed_ms = Some(0);

    let intervals = vec![
        interval_record_with_class(100, 6201, "game-main", TaskClass::Game, 0.0, 700_000),
        interval_record_with_class(
            100,
            6202,
            "gnome-shell",
            TaskClass::Compositor,
            0.0,
            800_000,
        ),
    ];
    let gpu_samples = vec![
        gpu_sample(90, "card0", "renderD128", 81, 1860, 135_000_000),
        gpu_sample(120, "card0", "renderD128", 84, 1880, 139_000_000),
    ];
    let frame_events = vec![
        FrameEvent {
            elapsed_ms: 90,
            frametime_ms: 16.7,
        },
        FrameEvent {
            elapsed_ms: 107,
            frametime_ms: 19.2,
        },
        FrameEvent {
            elapsed_ms: 124,
            frametime_ms: 16.8,
        },
    ];

    apply_artifact_counts(
        &mut session,
        &FixtureArtifacts {
            intervals,
            gpu_samples,
            frame_events,
            display_topology: Some(display_topology_snapshot("NVIDIA", "nvidia", "GNOME")),
            ..Default::default()
        },
    )
}

pub(in crate::test_fixture_builder) fn real_intel_kwin_cpu_bound_fixture()
-> (SessionFile, FixtureArtifacts) {
    let spikes = vec![
        spike_event(6301, TaskClass::Game, "game-main", 5_500_000, 0),
        spike_event(
            6302,
            TaskClass::GameWorkerThread,
            "worker",
            4_500_000,
            250_000,
        ),
        spike_event(
            6303,
            TaskClass::Helper,
            "compile-helper",
            3_900_000,
            500_000,
        ),
    ];
    let intervals = vec![
        interval_record_with_class(100, 6301, "game-main", TaskClass::Game, 78.0, 5_500_000),
        interval_record_with_class(
            100,
            6302,
            "worker",
            TaskClass::GameWorkerThread,
            74.0,
            4_500_000,
        ),
        interval_record_with_class(
            100,
            6303,
            "compile-helper",
            TaskClass::Helper,
            81.0,
            3_900_000,
        ),
    ];

    let mut session = platform_session("real_intel_kwin_cpu_bound", "Intel", "i915", "KWin");
    session.config.tree_roots = vec![6300];
    apply_spike_session_fields(&mut session, &spikes);
    apply_artifact_counts(
        &mut session,
        &FixtureArtifacts {
            spikes,
            intervals,
            display_topology: Some(display_topology_snapshot("Intel", "i915", "KWin")),
            ..Default::default()
        },
    )
}

pub(in crate::test_fixture_builder) fn real_amd_gamescope_gpu_bound_fixture()
-> (SessionFile, FixtureArtifacts) {
    let spikes = vec![
        spike_event(6401, TaskClass::Game, "game-main", 2_100_000, 0),
        spike_event(
            6402,
            TaskClass::GameRenderThread,
            "render",
            1_900_000,
            250_000,
        ),
        spike_event(6403, TaskClass::GameScope, "gamescope", 1_700_000, 500_000),
    ];
    let intervals = vec![
        interval_record_with_class(100, 6401, "game-main", TaskClass::Game, 1.0, 2_100_000),
        interval_record_with_class(
            100,
            6402,
            "render",
            TaskClass::GameRenderThread,
            1.0,
            1_900_000,
        ),
        interval_record_with_class(100, 6403, "gamescope", TaskClass::GameScope, 1.0, 1_700_000),
    ];
    let gpu_samples = vec![
        gpu_sample(84, "card0", "renderD128", 95, 2520, 205_000_000),
        gpu_sample(100, "card0", "renderD128", 99, 2550, 218_000_000),
        gpu_sample(117, "card0", "renderD128", 98, 2530, 216_000_000),
    ];
    let frame_events = vec![
        FrameEvent {
            elapsed_ms: 84,
            frametime_ms: 16.6,
        },
        FrameEvent {
            elapsed_ms: 100,
            frametime_ms: 58.0,
        },
        FrameEvent {
            elapsed_ms: 117,
            frametime_ms: 16.8,
        },
    ];

    let mut session =
        platform_session("real_amd_gamescope_gpu_bound", "AMD", "amdgpu", "Gamescope");
    session.config.tree_roots = vec![6400];
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
            display_topology: Some(display_topology_snapshot("AMD", "amdgpu", "Gamescope")),
            ..Default::default()
        },
    )
}

pub(in crate::test_fixture_builder) fn real_nvidia_kwin_irq_overlap_fixture()
-> (SessionFile, FixtureArtifacts) {
    let spikes = vec![
        spike_event(6501, TaskClass::Unknown, "game-worker-a", 6_200_000, 0),
        spike_event(
            6502,
            TaskClass::Unknown,
            "game-worker-b",
            4_600_000,
            250_000,
        ),
        spike_event(
            6503,
            TaskClass::Unknown,
            "game-worker-c",
            4_100_000,
            500_000,
        ),
    ];
    let intervals = vec![
        interval_record_with_class(
            100,
            6501,
            "game-worker-a",
            TaskClass::Unknown,
            3.0,
            6_200_000,
        ),
        interval_record_with_class(
            100,
            6502,
            "game-worker-b",
            TaskClass::Unknown,
            2.0,
            4_600_000,
        ),
        interval_record_with_class(
            100,
            6503,
            "game-worker-c",
            TaskClass::Unknown,
            2.0,
            4_100_000,
        ),
    ];
    let irq_events = vec![
        IrqEventRecord {
            elapsed_ms: Some(100),
            irq: 181,
            cpu: 4,
            enter_ns: 98_750_000,
            exit_ns: 104_750_000,
            duration_ns: 6_000_000,
        },
        IrqEventRecord {
            elapsed_ms: Some(170),
            irq: 182,
            cpu: 7,
            enter_ns: 170_000_000,
            exit_ns: 171_000_000,
            duration_ns: 1_000_000,
        },
    ];

    let mut session = platform_session("real_nvidia_kwin_irq_overlap", "NVIDIA", "nvidia", "KWin");
    session.config.tree_roots = vec![6500];
    session.config.irq_latency = true;
    session.config.irqs = vec![181, 182];
    session.core.metadata = crate::metadata::SystemMetadata {
        irq_lines: vec![crate::irq_inspect::IrqLine {
            irq: "181".to_owned(),
            counts_by_cpu: vec![0, 0, 0, 42],
            total: 42,
            kind: "PCI-MSI".to_owned(),
            name: "524288-edge nvidia".to_owned(),
            raw: "181: 0 0 0 42 PCI-MSI 524288-edge nvidia".to_owned(),
        }],
        ..Default::default()
    };
    apply_spike_session_fields(&mut session, &spikes);
    apply_artifact_counts(
        &mut session,
        &FixtureArtifacts {
            spikes,
            intervals,
            irq_events,
            display_topology: Some(display_topology_snapshot("NVIDIA", "nvidia", "KWin")),
            ..Default::default()
        },
    )
}

pub(in crate::test_fixture_builder) fn real_intel_sway_compositor_delay_fixture()
-> (SessionFile, FixtureArtifacts) {
    let spikes = vec![
        spike_event(6601, TaskClass::Compositor, "sway", 5_400_000, 0),
        spike_event(6602, TaskClass::Game, "game-main", 2_200_000, 250_000),
        spike_event(
            6603,
            TaskClass::Helper,
            "present-helper",
            1_900_000,
            500_000,
        ),
    ];
    let intervals = vec![
        interval_record_with_class(100, 6601, "sway", TaskClass::Compositor, 1.0, 5_400_000),
        interval_record_with_class(100, 6602, "game-main", TaskClass::Game, 1.0, 2_200_000),
        interval_record_with_class(
            100,
            6603,
            "present-helper",
            TaskClass::Helper,
            1.0,
            1_900_000,
        ),
    ];
    let frame_events = vec![
        FrameEvent {
            elapsed_ms: 84,
            frametime_ms: 16.6,
        },
        FrameEvent {
            elapsed_ms: 100,
            frametime_ms: 45.0,
        },
        FrameEvent {
            elapsed_ms: 117,
            frametime_ms: 16.7,
        },
    ];

    let mut session = platform_session("real_intel_sway_compositor_delay", "Intel", "i915", "Sway");
    session.config.tree_roots = vec![6600];
    session.core.mangohud_first_frame_monotonic_ns = Some(0);
    session.core.mangohud_first_frame_raw_elapsed_ms = Some(0);
    apply_spike_session_fields(&mut session, &spikes);
    apply_artifact_counts(
        &mut session,
        &FixtureArtifacts {
            spikes,
            intervals,
            frame_events,
            display_topology: Some(display_topology_snapshot("Intel", "i915", "Sway")),
            ..Default::default()
        },
    )
}

fn platform_session(name: &str, gpu_vendor: &str, driver: &str, compositor: &str) -> SessionFile {
    let mut session = base_session(name);
    session.config.display_topology = true;
    session.config.hwmon_drm_card = Some("card0".to_owned());
    session.core.display_path = Some(DisplayPathMetadata {
        label: Some(format!("{gpu_vendor}-{compositor}-sanitized")),
        render_gpu: Some(gpu_vendor.to_owned()),
        scanout_gpu: Some(gpu_vendor.to_owned()),
        connector: Some("DP-1".to_owned()),
        render_card: Some("card0".to_owned()),
        render_render_node: Some("renderD128".to_owned()),
        render_driver: Some(driver.to_owned()),
        scanout_card: Some("card0".to_owned()),
        scanout_driver: Some(driver.to_owned()),
        is_cross_gpu: Some(false),
        session_type: Some("wayland".to_owned()),
        compositor: Some(compositor.to_owned()),
        topology_confidence: Some("sanitized-real".to_owned()),
        topology_warnings: Vec::new(),
    });
    session
}

fn display_topology_snapshot(
    gpu_vendor: &str,
    driver: &str,
    compositor: &str,
) -> crate::display_topology::DisplayTopologySnapshot {
    crate::display_topology::DisplayTopologySnapshot {
        collected_at_elapsed_ms: Some(0),
        session_type: Some("wayland".to_owned()),
        compositor: Some(crate::display_topology::CompositorInfo {
            name: compositor.to_ascii_lowercase(),
            pid: Some(6000),
        }),
        drm_devices: vec![crate::display_topology::DrmDeviceInfo {
            card: "card0".to_owned(),
            render_node: Some("renderD128".to_owned()),
            driver: Some(driver.to_owned()),
            vendor_id: Some(
                match gpu_vendor {
                    "AMD" => "0x1002",
                    "NVIDIA" => "0x10de",
                    "Intel" => "0x8086",
                    _ => "0xffff",
                }
                .to_owned(),
            ),
            device_id: Some("0x0000".to_owned()),
            pci_slot: Some("0000:01:00.0".to_owned()),
            boot_vga: Some(true),
            hwmon_paths: Vec::new(),
        }],
        connectors: vec![crate::display_topology::ConnectorInfo {
            card: "card0".to_owned(),
            name: "DP-1".to_owned(),
            status: Some("connected".to_owned()),
            enabled: Some("enabled".to_owned()),
            modes: vec!["2560x1440@144".to_owned()],
            edid_hash: Some("fixture-edid".to_owned()),
        }],
        guessed_path: Some(crate::display_topology::DisplayPathGuess {
            render_card: Some("card0".to_owned()),
            render_driver: Some(driver.to_owned()),
            scanout_card: Some("card0".to_owned()),
            scanout_driver: Some(driver.to_owned()),
            connector: Some("DP-1".to_owned()),
            is_cross_gpu: Some(false),
            confidence: "sanitized-real".to_owned(),
            reasons: vec!["sanitized real validation matrix fixture".to_owned()],
        }),
        warnings: Vec::new(),
    }
}

fn gpu_sample(
    elapsed_ms: u64,
    drm_card: &str,
    render_node: &str,
    busy: u32,
    clock_mhz: u32,
    power_microwatts: u64,
) -> GpuSample {
    GpuSample {
        elapsed_ms,
        drm_card: Some(drm_card.to_owned()),
        render_node: Some(render_node.to_owned()),
        gpu_busy_percent: Some(busy),
        vram_used_bytes: Some(2_000_000_000),
        vram_total_bytes: Some(8_000_000_000),
        vram_used_percent: Some(25),
        gpu_clock_mhz: Some(clock_mhz),
        mem_clock_mhz: Some(7_000),
        temp_millidegrees: Some(65_000),
        power_microwatts: Some(power_microwatts),
        power_limit_reason: None,
    }
}
