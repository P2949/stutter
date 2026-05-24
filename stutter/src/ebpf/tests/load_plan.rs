//! Pure loader-plan tests that do not require eBPF privileges.

use std::path::PathBuf;

use crate::{
    drm_tracepoints,
    ebpf::{
        load_plan::build_loader_plan,
        model::{BlockIoCorrelationBasis, EbpfMapSizing},
        preflight::TracepointAvailability,
    },
};

fn attach_test_tracepoints() -> TracepointAvailability {
    TracepointAvailability {
        sched_wakeup_new: true,
        sched_migrate_task: true,
        cpu_frequency: false,
        sched_stat_wait: false,
        irq_handler: false,
        block_rq: false,
        block_rq_has_rwbs: false,
        block_rq_key_offset: None,
        block_rq_issue_nr_sector_offset: None,
        block_rq_issue_rwbs_offset: None,
        block_rq_complete_nr_sector_offset: None,
        block_rq_complete_rwbs_offset: None,
        kms: drm_tracepoints::KmsTracepointAvailability::unavailable(),
        drm_fence: None,
        sched_process_exit: true,
        sched_process_exec: true,
    }
}

fn test_map_sizing() -> EbpfMapSizing {
    EbpfMapSizing {
        events_ringbuf_bytes: 256 * 1024,
        wakeup_data_entries: 8192,
        locked_memory_limit_bytes: Some(64 * 1024 * 1024),
        available_memory_bytes: Some(8 * 1024 * 1024 * 1024),
    }
}

fn fence_format(
    category: &str,
    name: &str,
) -> crate::drm_fence_tracepoints::DrmFenceTracepointFormat {
    crate::drm_fence_tracepoints::parse_tracepoint_format(
        category,
        name,
        "\
field:u64 context;\toffset:8;\tsize:8;\tsigned:0;
field:u64 seqno;\toffset:16;\tsize:8;\tsigned:0;
field:char timeline[32];\toffset:24;\tsize:32;\tsigned:0;
",
    )
}

fn fence_discovery() -> crate::drm_fence_tracepoints::DrmFenceTracepointDiscovery {
    crate::drm_fence_tracepoints::DrmFenceTracepointDiscovery {
        events_root: PathBuf::from("/test/events"),
        supported_profile: "test".to_owned(),
        categories: vec![crate::drm_fence_tracepoints::DrmFenceTracepointCategory {
            category: "dma_fence".to_owned(),
            status: "available".to_owned(),
            tracepoints: vec![
                fence_format("dma_fence", "dma_fence_wait_start"),
                fence_format("dma_fence", "dma_fence_wait_done"),
                fence_format("dma_fence", "dma_fence_signal"),
            ],
            warnings: Vec::new(),
        }],
    }
}

#[test]
fn loader_plan_resizes_events_and_wakeup_maps_together() {
    let tracepoints = attach_test_tracepoints();
    let plan = build_loader_plan(&tracepoints, test_map_sizing());

    assert_eq!(plan.map_entries("EVENTS"), Some(256 * 1024));
    assert_eq!(plan.map_entries("WAKEUP_DATA"), Some(8192));
    assert_eq!(plan.map_entries("WAKEUP_CONSUMED"), Some(8192));
}

#[test]
fn loader_plan_uses_request_pointer_block_correlation_when_key_offset_exists() {
    let mut tracepoints = attach_test_tracepoints();
    tracepoints.block_rq = true;
    tracepoints.block_rq_has_rwbs = true;
    tracepoints.block_rq_key_offset = Some(40);
    tracepoints.block_rq_issue_nr_sector_offset = Some(24);
    tracepoints.block_rq_issue_rwbs_offset = Some(32);
    tracepoints.block_rq_complete_nr_sector_offset = Some(24);
    tracepoints.block_rq_complete_rwbs_offset = Some(32);

    let plan = build_loader_plan(&tracepoints, test_map_sizing());

    assert_eq!(
        plan.block_io_correlation_basis,
        BlockIoCorrelationBasis::RequestPointer
    );
    assert_eq!(plan.global_value("BLOCK_RQ_KEY_OFFSET"), Some(40));
    assert_eq!(
        plan.global_value("BLOCK_RQ_ISSUE_NR_SECTOR_OFFSET"),
        Some(24)
    );
    assert_eq!(plan.global_value("BLOCK_RQ_ISSUE_RWBS_OFFSET"), Some(32));
    assert_eq!(
        plan.global_value("BLOCK_RQ_COMPLETE_NR_SECTOR_OFFSET"),
        Some(24)
    );
    assert_eq!(plan.global_value("BLOCK_RQ_COMPLETE_RWBS_OFFSET"), Some(32));
}

#[test]
fn loader_plan_uses_dev_sector_block_correlation_without_request_pointer() {
    let mut tracepoints = attach_test_tracepoints();
    tracepoints.block_rq = true;
    tracepoints.block_rq_key_offset = None;
    tracepoints.block_rq_issue_nr_sector_offset = Some(24);
    tracepoints.block_rq_complete_nr_sector_offset = Some(24);

    let plan = build_loader_plan(&tracepoints, test_map_sizing());

    assert_eq!(
        plan.block_io_correlation_basis,
        BlockIoCorrelationBasis::DevSector
    );
    assert_eq!(plan.global_value("BLOCK_RQ_KEY_OFFSET"), None);
}

#[test]
fn loader_plan_disables_block_correlation_when_block_probe_unavailable() {
    let mut tracepoints = attach_test_tracepoints();
    tracepoints.block_rq = false;
    tracepoints.block_rq_key_offset = Some(40);

    let plan = build_loader_plan(&tracepoints, test_map_sizing());

    assert_eq!(
        plan.block_io_correlation_basis,
        BlockIoCorrelationBasis::Disabled
    );
    assert_eq!(plan.global_value("BLOCK_RQ_KEY_OFFSET"), None);
}

#[test]
fn loader_plan_records_generic_drm_kms_offsets() {
    let mut tracepoints = attach_test_tracepoints();
    tracepoints.kms = drm_tracepoints::KmsTracepointAvailability {
        pageflip_request: None,
        pageflip_done: None,
        vblank_event: Some(drm_tracepoints::parse_drm_tracepoint_format(
            "drm",
            "drm_vblank_event",
            "\
field:unsigned int crtc_id;\toffset:8;\tsize:4;\tsigned:0;
field:unsigned int pipe;\toffset:12;\tsize:4;\tsigned:0;
field:u64 sequence;\toffset:16;\tsize:8;\tsigned:0;
",
        )),
        atomic_commit: None,
        provider: drm_tracepoints::KmsTracepointProvider::GenericDrm,
        generic_drm: Vec::new(),
        i915: Vec::new(),
        amdgpu: Vec::new(),
        warnings: Vec::new(),
    };

    let plan = build_loader_plan(&tracepoints, test_map_sizing());

    assert_eq!(plan.global_value("DRM_VBLANK_CRTC_OFFSET"), Some(8));
    assert_eq!(plan.global_value("DRM_VBLANK_PIPE_OFFSET"), Some(12));
    assert_eq!(plan.global_value("DRM_VBLANK_SEQUENCE_OFFSET"), Some(16));
    assert_eq!(plan.global_value("DRM_VBLANK_SEQUENCE_SIZE"), Some(8));
}

#[test]
fn loader_plan_records_drm_fence_offsets() {
    let mut tracepoints = attach_test_tracepoints();
    tracepoints.drm_fence = Some(fence_discovery());

    let plan = build_loader_plan(&tracepoints, test_map_sizing());

    assert_eq!(
        plan.global_value("DRM_FENCE_WAIT_START_CONTEXT_OFFSET"),
        Some(8)
    );
    assert_eq!(
        plan.global_value("DRM_FENCE_WAIT_START_SEQNO_OFFSET"),
        Some(16)
    );
    assert_eq!(
        plan.global_value("DRM_FENCE_WAIT_START_TIMELINE_OFFSET"),
        Some(24)
    );
    assert_eq!(
        plan.global_value("DRM_FENCE_SIGNAL_CONTEXT_OFFSET"),
        Some(8)
    );
    assert!(plan.drm_fence_offsets.is_some());
}
