//! Synthetic display-path validation fixture constructors.

use super::{builders::*, *};

pub(in crate::test_fixture_builder) fn direct_gpu_clean_fixture() -> (SessionFile, FixtureArtifacts)
{
    display_path_fixture(DisplayPathFixtureInput {
        name: "direct_gpu_clean",
        is_cross_gpu: false,
        frame_outlier: false,
        kms_duration_ns: Some(250_000),
        fence_duration_ns: None,
        wayland_zero_copy: Some(true),
        wayland_commit_to_present_ns: Some(500_000),
        wayland_flags: Vec::new(),
        dmabuf_copy_required: None,
        dmabuf_reason: None,
        igpu_blitter_busy: None,
        amdgpu_gfx_busy: Some(45.0),
    })
}

pub(in crate::test_fixture_builder) fn uhd630_cross_gpu_fence_wait_fixture()
-> (SessionFile, FixtureArtifacts) {
    display_path_fixture(DisplayPathFixtureInput {
        name: "uhd630_cross_gpu_fence_wait",
        is_cross_gpu: true,
        frame_outlier: true,
        kms_duration_ns: Some(2_200_000),
        fence_duration_ns: Some(2_400_000),
        wayland_zero_copy: Some(false),
        wayland_commit_to_present_ns: Some(2_100_000),
        wayland_flags: vec!["format_modifier_mismatch"],
        dmabuf_copy_required: Some(true),
        dmabuf_reason: Some("modifier_mismatch"),
        igpu_blitter_busy: Some(40.0),
        amdgpu_gfx_busy: Some(55.0),
    })
}

pub(in crate::test_fixture_builder) fn uhd630_composited_blitter_fixture()
-> (SessionFile, FixtureArtifacts) {
    display_path_fixture(DisplayPathFixtureInput {
        name: "uhd630_composited_blitter",
        is_cross_gpu: true,
        frame_outlier: true,
        kms_duration_ns: Some(700_000),
        fence_duration_ns: None,
        wayland_zero_copy: Some(false),
        wayland_commit_to_present_ns: Some(1_800_000),
        wayland_flags: vec!["composited"],
        dmabuf_copy_required: Some(false),
        dmabuf_reason: Some("composited"),
        igpu_blitter_busy: Some(75.0),
        amdgpu_gfx_busy: Some(40.0),
    })
}

pub(in crate::test_fixture_builder) fn uhd630_kms_delay_fixture() -> (SessionFile, FixtureArtifacts)
{
    display_path_fixture(DisplayPathFixtureInput {
        name: "uhd630_kms_delay",
        is_cross_gpu: true,
        frame_outlier: true,
        kms_duration_ns: Some(3_200_000),
        fence_duration_ns: None,
        wayland_zero_copy: Some(true),
        wayland_commit_to_present_ns: Some(600_000),
        wayland_flags: Vec::new(),
        dmabuf_copy_required: None,
        dmabuf_reason: None,
        igpu_blitter_busy: None,
        amdgpu_gfx_busy: Some(35.0),
    })
}

pub(in crate::test_fixture_builder) fn wayland_zero_copy_good_fixture()
-> (SessionFile, FixtureArtifacts) {
    display_path_fixture(DisplayPathFixtureInput {
        name: "wayland_zero_copy_good",
        is_cross_gpu: false,
        frame_outlier: false,
        kms_duration_ns: Some(300_000),
        fence_duration_ns: None,
        wayland_zero_copy: Some(true),
        wayland_commit_to_present_ns: Some(450_000),
        wayland_flags: Vec::new(),
        dmabuf_copy_required: Some(false),
        dmabuf_reason: None,
        igpu_blitter_busy: None,
        amdgpu_gfx_busy: Some(50.0),
    })
}

pub(in crate::test_fixture_builder) fn dmabuf_modifier_mismatch_fixture()
-> (SessionFile, FixtureArtifacts) {
    display_path_fixture(DisplayPathFixtureInput {
        name: "dmabuf_modifier_mismatch",
        is_cross_gpu: true,
        frame_outlier: true,
        kms_duration_ns: Some(800_000),
        fence_duration_ns: None,
        wayland_zero_copy: None,
        wayland_commit_to_present_ns: None,
        wayland_flags: Vec::new(),
        dmabuf_copy_required: Some(true),
        dmabuf_reason: Some("modifier_mismatch"),
        igpu_blitter_busy: None,
        amdgpu_gfx_busy: Some(45.0),
    })
}

pub(in crate::test_fixture_builder) fn missing_evidence_unknown_fixture()
-> (SessionFile, FixtureArtifacts) {
    let mut session = base_session("missing_evidence_unknown");
    session.tasks = vec![task_for_fixture(5901, TaskClass::Game, "Main", 4, 700_000)];
    session.core.active_target_pids_count = 1;
    session.core.active_expanded_tasks = vec![5901];

    apply_artifact_counts(
        &mut session,
        &FixtureArtifacts {
            intervals: vec![interval_record_with_class(
                100,
                5901,
                "Main",
                TaskClass::Game,
                0.0,
                700_000,
            )],
            ..Default::default()
        },
    )
}

struct DisplayPathFixtureInput<'a> {
    name: &'a str,
    is_cross_gpu: bool,
    frame_outlier: bool,
    kms_duration_ns: Option<u64>,
    fence_duration_ns: Option<u64>,
    wayland_zero_copy: Option<bool>,
    wayland_commit_to_present_ns: Option<u64>,
    wayland_flags: Vec<&'a str>,
    dmabuf_copy_required: Option<bool>,
    dmabuf_reason: Option<&'a str>,
    igpu_blitter_busy: Option<f64>,
    amdgpu_gfx_busy: Option<f64>,
}

fn display_path_fixture(input: DisplayPathFixtureInput<'_>) -> (SessionFile, FixtureArtifacts) {
    let mut session = base_session(input.name);
    session.config.hwmon = true;
    session.config.kms_timing = input.kms_duration_ns.is_some();
    session.config.kms_card = Some(if input.is_cross_gpu {
        "card0".to_owned()
    } else {
        "card1".to_owned()
    });
    session.config.kms_connector = Some("DP-1".to_owned());
    session.config.drm_fence_latency = input.fence_duration_ns.is_some();
    session.config.drm_fence_render_card = Some("card1".to_owned());
    session.config.drm_fence_display_card = Some(if input.is_cross_gpu {
        "card0".to_owned()
    } else {
        "card1".to_owned()
    });
    session.config.drm_fence_driver = Some(if input.is_cross_gpu {
        "i915".to_owned()
    } else {
        "amdgpu".to_owned()
    });
    session.config.wayland_presentation = input.wayland_zero_copy.is_some();
    session.config.dmabuf_tracking = input.dmabuf_copy_required.is_some();
    session.config.gpu_engine_sampling =
        input.igpu_blitter_busy.is_some() || input.amdgpu_gfx_busy.is_some();
    session.config.display_topology = true;
    session.core.display_path = Some(display_path_metadata_for(input.is_cross_gpu));
    session.tasks = vec![task_for_fixture(5901, TaskClass::Game, "Main", 4, 700_000)];
    session.core.active_target_pids_count = 1;
    session.core.active_expanded_tasks = vec![5901];
    session.core.mangohud_first_frame_monotonic_ns = Some(0);
    session.core.mangohud_first_frame_raw_elapsed_ms = Some(0);

    let intervals = vec![interval_record_with_class(
        100,
        5901,
        "Main",
        TaskClass::Game,
        0.0,
        700_000,
    )];
    let frame_events = display_path_frames(input.frame_outlier);
    let kms_flip_events = input
        .kms_duration_ns
        .map(display_path_kms_event)
        .into_iter()
        .collect::<Vec<_>>();
    let drm_fence_events = input
        .fence_duration_ns
        .map(display_path_fence_event)
        .into_iter()
        .collect::<Vec<_>>();
    let wayland_presentation_events = input
        .wayland_zero_copy
        .map(|zero_copy| {
            display_path_wayland_event(
                zero_copy,
                input.wayland_commit_to_present_ns.unwrap_or(500_000),
                &input.wayland_flags,
            )
        })
        .into_iter()
        .collect::<Vec<_>>();
    let dmabuf_events = input
        .dmabuf_copy_required
        .map(|copy_required| {
            display_path_dmabuf_event(copy_required, input.dmabuf_reason, input.is_cross_gpu)
        })
        .into_iter()
        .collect::<Vec<_>>();
    let gpu_engine_samples = display_path_engine_samples(
        input.igpu_blitter_busy,
        input.amdgpu_gfx_busy,
        input.frame_outlier,
    );
    let display_topology = Some(display_path_topology(input.is_cross_gpu));

    apply_artifact_counts(
        &mut session,
        &FixtureArtifacts {
            intervals,
            frame_events,
            kms_flip_events,
            drm_fence_events,
            wayland_presentation_events,
            dmabuf_events,
            gpu_engine_samples,
            display_topology,
            ..Default::default()
        },
    )
}

fn display_path_metadata_for(is_cross_gpu: bool) -> DisplayPathMetadata {
    let scanout_card = if is_cross_gpu { "card0" } else { "card1" };
    let scanout_driver = if is_cross_gpu { "i915" } else { "amdgpu" };
    DisplayPathMetadata {
        label: Some(if is_cross_gpu {
            "cross-gpu".to_owned()
        } else {
            "direct-scanout-gpu".to_owned()
        }),
        render_gpu: Some("amdgpu".to_owned()),
        scanout_gpu: Some(scanout_driver.to_owned()),
        connector: Some("DP-1".to_owned()),
        render_card: Some("card1".to_owned()),
        render_render_node: Some("/dev/dri/renderD129".to_owned()),
        render_driver: Some("amdgpu".to_owned()),
        scanout_card: Some(scanout_card.to_owned()),
        scanout_driver: Some(scanout_driver.to_owned()),
        is_cross_gpu: Some(is_cross_gpu),
        session_type: Some("wayland".to_owned()),
        compositor: Some("gamescope".to_owned()),
        topology_confidence: Some("high".to_owned()),
        topology_warnings: Vec::new(),
    }
}

fn display_path_topology(is_cross_gpu: bool) -> crate::display_topology::DisplayTopologySnapshot {
    let scanout_card = if is_cross_gpu { "card0" } else { "card1" };
    let scanout_driver = if is_cross_gpu { "i915" } else { "amdgpu" };
    crate::display_topology::DisplayTopologySnapshot {
        collected_at_elapsed_ms: Some(0),
        session_type: Some("wayland".to_owned()),
        compositor: Some(crate::display_topology::CompositorInfo {
            name: "gamescope".to_owned(),
            pid: Some(5900),
        }),
        drm_devices: vec![
            crate::display_topology::DrmDeviceInfo {
                card: "card0".to_owned(),
                render_node: Some("renderD128".to_owned()),
                driver: Some("i915".to_owned()),
                vendor_id: Some("0x8086".to_owned()),
                device_id: Some("0x3e92".to_owned()),
                pci_slot: Some("0000:00:02.0".to_owned()),
                boot_vga: Some(true),
                hwmon_paths: Vec::new(),
            },
            crate::display_topology::DrmDeviceInfo {
                card: "card1".to_owned(),
                render_node: Some("renderD129".to_owned()),
                driver: Some("amdgpu".to_owned()),
                vendor_id: Some("0x1002".to_owned()),
                device_id: Some("0x744c".to_owned()),
                pci_slot: Some("0000:03:00.0".to_owned()),
                boot_vga: Some(false),
                hwmon_paths: Vec::new(),
            },
        ],
        connectors: vec![crate::display_topology::ConnectorInfo {
            card: scanout_card.to_owned(),
            name: "DP-1".to_owned(),
            status: Some("connected".to_owned()),
            enabled: Some("enabled".to_owned()),
            modes: vec!["2560x1440@144".to_owned()],
            edid_hash: Some("fixture-edid".to_owned()),
        }],
        guessed_path: Some(crate::display_topology::DisplayPathGuess {
            render_card: Some("card1".to_owned()),
            render_driver: Some("amdgpu".to_owned()),
            scanout_card: Some(scanout_card.to_owned()),
            scanout_driver: Some(scanout_driver.to_owned()),
            connector: Some("DP-1".to_owned()),
            is_cross_gpu: Some(is_cross_gpu),
            confidence: "high".to_owned(),
            reasons: vec!["synthetic display-path validation fixture".to_owned()],
        }),
        warnings: Vec::new(),
    }
}

fn display_path_frames(outlier: bool) -> Vec<FrameEvent> {
    vec![
        FrameEvent {
            elapsed_ms: 84,
            frametime_ms: 16.6,
        },
        FrameEvent {
            elapsed_ms: 100,
            frametime_ms: if outlier { 45.0 } else { 16.7 },
        },
        FrameEvent {
            elapsed_ms: 117,
            frametime_ms: 16.7,
        },
        FrameEvent {
            elapsed_ms: 134,
            frametime_ms: 16.6,
        },
    ]
}

fn display_path_kms_event(duration_ns: u64) -> KmsFlipEventRecord {
    KmsFlipEventRecord {
        elapsed_ms: 100,
        timestamp_ns: 100_000_000,
        source: "i915".to_owned(),
        card: Some("card0".to_owned()),
        driver: Some("i915".to_owned()),
        crtc_id: Some(42),
        connector: Some("DP-1".to_owned()),
        event_kind: "pageflip_interval".to_owned(),
        sequence: Some(77),
        request_ns: Some(99_000_000),
        done_ns: Some(99_000_000 + duration_ns),
        duration_ns: Some(duration_ns),
        flags: Vec::new(),
        confidence: "high".to_owned(),
    }
}

fn display_path_fence_event(duration_ns: u64) -> DrmFenceEventRecord {
    DrmFenceEventRecord {
        elapsed_ms: 100,
        timestamp_ns: 100_000_000,
        source: "i915".to_owned(),
        event_kind: "wait_interval".to_owned(),
        driver: Some("i915".to_owned()),
        card: Some("card0".to_owned()),
        gpu_role: Some("display".to_owned()),
        pid: Some(5901.into()),
        tid: Some(5901.into()),
        comm: Some("Main".to_owned()),
        context: Some(7),
        seqno: Some(11),
        timeline_hash: None,
        wait_start_ns: Some(98_500_000),
        wait_done_ns: Some(98_500_000 + duration_ns),
        signal_ns: Some(98_000_000),
        duration_ns: Some(duration_ns),
        exporter_driver: Some("amdgpu".to_owned()),
        importer_driver: Some("i915".to_owned()),
        correlation_basis: "context_seqno".to_owned(),
        confidence: "high".to_owned(),
    }
}

fn display_path_wayland_event(
    zero_copy: bool,
    commit_to_present_ns: u64,
    flags: &[&str],
) -> WaylandPresentationEventRecord {
    WaylandPresentationEventRecord {
        elapsed_ms: 100,
        source: "gamescope".to_owned(),
        app_id: Some("synthetic-game".to_owned()),
        surface_role: Some("game".to_owned()),
        commit_ns: Some(98_000_000),
        presented_ns: Some(98_000_000 + commit_to_present_ns),
        commit_to_present_ns: Some(commit_to_present_ns),
        output_name: Some("DP-1".to_owned()),
        refresh_ns: Some(6_944_444),
        sequence: Some(77),
        zero_copy: Some(zero_copy),
        discarded: false,
        flags: flags.iter().map(|flag| (*flag).to_owned()).collect(),
        confidence: "high".to_owned(),
    }
}

fn display_path_dmabuf_event(
    copy_required: bool,
    reason: Option<&str>,
    is_cross_gpu: bool,
) -> DmaBufEventRecord {
    DmaBufEventRecord {
        elapsed_ms: 100,
        source: "gamescope".to_owned(),
        app_id: Some("synthetic-game".to_owned()),
        surface_role: Some("game".to_owned()),
        output_name: Some("DP-1".to_owned()),
        width: Some(2560),
        height: Some(1440),
        format: Some("XRGB8888".to_owned()),
        modifier: Some(if copy_required { "LINEAR" } else { "AFBC" }.to_owned()),
        modifier_name: None,
        planes: Some(1),
        allocation_driver: Some("amdgpu".to_owned()),
        import_driver: Some(if is_cross_gpu { "i915" } else { "amdgpu" }.to_owned()),
        allocation_card: Some("card1".to_owned()),
        import_card: Some(if is_cross_gpu { "card0" } else { "card1" }.to_owned()),
        linear: Some(copy_required),
        scanout_capable: Some(!copy_required),
        zero_copy: Some(!copy_required),
        explicit_sync: Some(true),
        copy_required: Some(copy_required),
        reason: reason.map(str::to_owned),
        confidence: "medium".to_owned(),
    }
}

fn display_path_engine_samples(
    igpu_blitter_busy: Option<f64>,
    amdgpu_gfx_busy: Option<f64>,
    near_outlier: bool,
) -> Vec<GpuEngineSample> {
    let elapsed_ms = if near_outlier { 100 } else { 84 };
    let mut samples = Vec::new();
    if let Some(busy) = igpu_blitter_busy {
        samples.push(GpuEngineSample {
            elapsed_ms,
            drm_card: Some("card0".to_owned()),
            render_node: Some("renderD128".to_owned()),
            driver: Some("i915".to_owned()),
            engine: "bcs0".to_owned(),
            busy_percent: Some(busy),
            client_pid: None,
            client_comm: None,
            source: "hwmon".to_owned(),
            confidence: "low".to_owned(),
        });
    }
    if let Some(busy) = amdgpu_gfx_busy {
        samples.push(GpuEngineSample {
            elapsed_ms,
            drm_card: Some("card1".to_owned()),
            render_node: Some("renderD129".to_owned()),
            driver: Some("amdgpu".to_owned()),
            engine: "gfx".to_owned(),
            busy_percent: Some(busy),
            client_pid: None,
            client_comm: None,
            source: "hwmon".to_owned(),
            confidence: "low".to_owned(),
        });
    }
    samples
}
