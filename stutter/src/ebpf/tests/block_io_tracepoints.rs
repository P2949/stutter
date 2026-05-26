//! Tests for block I/O tracepoint format validation.
//!
//! Owns block tracepoint parser/offset regression tests. Does not own production tracepoint
//! parsing, preflight reporting, or eBPF attach logic.

use super::*;

fn write_block_format(events_root: &Path, tracepoint: &str, contents: &str) {
    let dir = events_root.join("block").join(tracepoint);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("format"), contents).unwrap();
}

fn block_rq_format(
    rq: Option<(u32, u32)>,
    nr_sector: Option<(u32, u32)>,
    rwbs: Option<(u32, u32)>,
) -> String {
    let mut format = String::from(
        "name: block_rq\nID: 1\nformat:\n\
         \tfield:unsigned short common_type;\toffset:0;\tsize:2;\tsigned:0;\n\
         \tfield:dev_t dev;\toffset:8;\tsize:4;\tsigned:0;\n\
         \tfield:sector_t sector;\toffset:16;\tsize:8;\tsigned:0;\n",
    );

    if let Some((offset, size)) = rq {
        format.push_str(&format!(
            "\tfield:void *rq;\toffset:{offset};\tsize:{size};\tsigned:0;\n"
        ));
    }

    if let Some((offset, size)) = nr_sector {
        format.push_str(&format!(
            "\tfield:unsigned int nr_sector;\toffset:{offset};\tsize:{size};\tsigned:0;\n"
        ));
    }

    if let Some((offset, size)) = rwbs {
        format.push_str(&format!(
            "\tfield:char rwbs[8];\toffset:{offset};\tsize:{size};\tsigned:0;\n"
        ));
    }

    format
}

#[test]
fn block_io_request_pointer_available_and_valid() {
    let temp = tempfile::tempdir().unwrap();
    let events_root = temp.path();

    let format = block_rq_format(Some((40, 8)), Some((48, 4)), Some((56, 8)));
    write_block_format(events_root, "block_rq_issue", &format);
    write_block_format(events_root, "block_rq_complete", &format);

    let offsets = validate_block_io_tracepoint_offsets(events_root);

    assert!(offsets.block_rq);
    assert!(offsets.block_rq_has_rwbs);
    assert_eq!(offsets.block_rq_key_offset, Some(40));
    assert_eq!(offsets.block_rq_issue_nr_sector_offset, Some(48));
    assert_eq!(offsets.block_rq_complete_nr_sector_offset, Some(48));
    assert_eq!(offsets.block_rq_issue_rwbs_offset, Some(56));
    assert_eq!(offsets.block_rq_complete_rwbs_offset, Some(56));
}

#[test]
fn block_io_request_pointer_absent_falls_back_to_dev_sector() {
    let temp = tempfile::tempdir().unwrap();
    let events_root = temp.path();

    let format = block_rq_format(None, Some((48, 4)), Some((56, 8)));
    write_block_format(events_root, "block_rq_issue", &format);
    write_block_format(events_root, "block_rq_complete", &format);

    let offsets = validate_block_io_tracepoint_offsets(events_root);

    assert!(offsets.block_rq);
    assert!(offsets.block_rq_has_rwbs);
    assert_eq!(offsets.block_rq_key_offset, None);
    assert_eq!(offsets.block_rq_issue_nr_sector_offset, Some(48));
    assert_eq!(offsets.block_rq_complete_nr_sector_offset, Some(48));
}

#[test]
fn block_io_rwbs_absent_keeps_rwbs_globals_unset() {
    let temp = tempfile::tempdir().unwrap();
    let events_root = temp.path();

    let format = block_rq_format(Some((40, 8)), Some((48, 4)), None);
    write_block_format(events_root, "block_rq_issue", &format);
    write_block_format(events_root, "block_rq_complete", &format);

    let offsets = validate_block_io_tracepoint_offsets(events_root);

    assert!(offsets.block_rq);
    assert!(!offsets.block_rq_has_rwbs);
    assert_eq!(offsets.block_rq_key_offset, Some(40));
    assert_eq!(offsets.block_rq_issue_rwbs_offset, None);
    assert_eq!(offsets.block_rq_complete_rwbs_offset, None);
}

#[test]
fn block_io_malformed_small_field_sizes_are_not_used() {
    let temp = tempfile::tempdir().unwrap();
    let events_root = temp.path();

    let format = block_rq_format(Some((40, 4)), Some((48, 2)), Some((56, 4)));
    write_block_format(events_root, "block_rq_issue", &format);
    write_block_format(events_root, "block_rq_complete", &format);

    let offsets = validate_block_io_tracepoint_offsets(events_root);

    assert!(offsets.block_rq);
    assert!(!offsets.block_rq_has_rwbs);
    assert_eq!(offsets.block_rq_key_offset, None);
    assert_eq!(offsets.block_rq_issue_nr_sector_offset, None);
    assert_eq!(offsets.block_rq_complete_nr_sector_offset, None);
    assert_eq!(offsets.block_rq_issue_rwbs_offset, None);
    assert_eq!(offsets.block_rq_complete_rwbs_offset, None);
}

#[test]
fn block_io_invalid_dev_sector_metadata_disables_block_rq() {
    let temp = tempfile::tempdir().unwrap();
    let events_root = temp.path();

    let bad_format = "name: block_rq\nID: 1\nformat:\n\
        \tfield:dev_t dev;\toffset:12;\tsize:4;\tsigned:0;\n\
        \tfield:sector_t sector;\toffset:16;\tsize:8;\tsigned:0;\n\
        \tfield:void *rq;\toffset:40;\tsize:8;\tsigned:0;\n\
        \tfield:unsigned int nr_sector;\toffset:48;\tsize:4;\tsigned:0;\n\
        \tfield:char rwbs[8];\toffset:56;\tsize:8;\tsigned:0;\n";

    write_block_format(events_root, "block_rq_issue", bad_format);
    write_block_format(events_root, "block_rq_complete", bad_format);

    let offsets = validate_block_io_tracepoint_offsets(events_root);

    assert!(!offsets.block_rq);
    assert_eq!(offsets.block_rq_key_offset, None);
    assert_eq!(offsets.block_rq_issue_nr_sector_offset, None);
    assert_eq!(offsets.block_rq_complete_nr_sector_offset, None);
    assert_eq!(offsets.block_rq_issue_rwbs_offset, None);
    assert_eq!(offsets.block_rq_complete_rwbs_offset, None);
}

#[test]
fn block_io_request_pointer_accepts_legacy_req_field_name() {
    let temp = tempfile::tempdir().unwrap();
    let events_root = temp.path();

    let format = "name: block_rq\nID: 1\nformat:\n\
        \tfield:dev_t dev;\toffset:8;\tsize:4;\tsigned:0;\n\
        \tfield:sector_t sector;\toffset:16;\tsize:8;\tsigned:0;\n\
        \tfield:void *req;\toffset:40;\tsize:8;\tsigned:0;\n\
        \tfield:unsigned int nr_sector;\toffset:48;\tsize:4;\tsigned:0;\n\
        \tfield:char rwbs[8];\toffset:56;\tsize:8;\tsigned:0;\n";

    write_block_format(events_root, "block_rq_issue", format);
    write_block_format(events_root, "block_rq_complete", format);

    let offsets = validate_block_io_tracepoint_offsets(events_root);

    assert!(offsets.block_rq);
    assert_eq!(offsets.block_rq_key_offset, Some(40));
}

#[test]
fn block_io_issue_complete_request_pointer_mismatch_uses_fallback() {
    let temp = tempfile::tempdir().unwrap();
    let events_root = temp.path();

    let issue_format = block_rq_format(Some((40, 8)), Some((48, 4)), Some((56, 8)));
    let complete_format = block_rq_format(Some((44, 8)), Some((48, 4)), Some((56, 8)));
    write_block_format(events_root, "block_rq_issue", &issue_format);
    write_block_format(events_root, "block_rq_complete", &complete_format);

    let offsets = validate_block_io_tracepoint_offsets(events_root);

    assert!(offsets.block_rq);
    assert!(offsets.block_rq_has_rwbs);
    assert_eq!(offsets.block_rq_key_offset, None);
    assert_eq!(offsets.block_rq_issue_nr_sector_offset, Some(48));
    assert_eq!(offsets.block_rq_complete_nr_sector_offset, Some(48));
}

#[test]
fn kms_provider_offsets_require_common_flip_identity() {
    let kms = crate::drm_tracepoints::KmsTracepointAvailability {
        pageflip_request: Some(crate::drm_tracepoints::parse_drm_tracepoint_format(
            "i915",
            "i915_flip_request",
            "field:unsigned int pipe;\toffset:8;\tsize:4;\tsigned:0;\n",
        )),
        pageflip_done: Some(crate::drm_tracepoints::parse_drm_tracepoint_format(
            "i915",
            "i915_flip_complete",
            "field:unsigned int pipe;\toffset:12;\tsize:4;\tsigned:0;\n\
             field:unsigned long sequence;\toffset:16;\tsize:8;\tsigned:0;\n",
        )),
        vblank_event: None,
        atomic_commit: None,
        provider: crate::drm_tracepoints::KmsTracepointProvider::I915,
        generic_drm: Vec::new(),
        i915: Vec::new(),
        amdgpu: Vec::new(),
        warnings: Vec::new(),
    };

    let offsets = kms_provider_tracepoint_offsets(&kms).unwrap();

    assert_eq!(offsets.request_pipe_offset, 8);
    assert_eq!(offsets.done_pipe_offset, 12);
    assert_eq!(offsets.done_sequence_offset, 16);
    assert_eq!(offsets.done_sequence_size, 8);

    let mut missing_identity = kms;
    missing_identity.pageflip_done = Some(crate::drm_tracepoints::parse_drm_tracepoint_format(
        "i915",
        "i915_flip_complete",
        "field:unsigned long sequence;\toffset:16;\tsize:8;\tsigned:0;\n",
    ));
    assert!(kms_provider_tracepoint_offsets(&missing_identity).is_none());
}

#[test]
fn drm_fence_offsets_preserve_vendor_provider_roles() {
    let discovery = crate::drm_fence_tracepoints::DrmFenceTracepointDiscovery {
        events_root: PathBuf::from("/tmp/events"),
        categories: vec![
            crate::drm_fence_tracepoints::DrmFenceTracepointCategory {
                category: "amdgpu".to_owned(),
                status: "available".to_owned(),
                tracepoints: vec![crate::drm_fence_tracepoints::parse_tracepoint_format(
                    "amdgpu",
                    "amdgpu_job_done",
                    "field:u64 context;\toffset:8;\tsize:8;\tsigned:0;\n\
                     field:u64 seqno;\toffset:16;\tsize:8;\tsigned:0;\n",
                )],
                warnings: Vec::new(),
            },
            crate::drm_fence_tracepoints::DrmFenceTracepointCategory {
                category: "i915".to_owned(),
                status: "available".to_owned(),
                tracepoints: vec![
                    crate::drm_fence_tracepoints::parse_tracepoint_format(
                        "i915",
                        "i915_request_wait_begin",
                        "field:u64 context;\toffset:24;\tsize:8;\tsigned:0;\n\
                         field:u64 seqno;\toffset:32;\tsize:8;\tsigned:0;\n",
                    ),
                    crate::drm_fence_tracepoints::parse_tracepoint_format(
                        "i915",
                        "i915_request_wait_end",
                        "field:u64 context;\toffset:40;\tsize:8;\tsigned:0;\n\
                         field:u64 seqno;\toffset:48;\tsize:8;\tsigned:0;\n",
                    ),
                ],
                warnings: Vec::new(),
            },
        ],
        supported_profile: "amdgpu+i915 partial".to_owned(),
    };

    let offsets = drm_fence_tracepoint_offsets(&discovery).unwrap();

    assert!(offsets.has_wait_interval);
    assert!(offsets.has_signal);
    assert_eq!(offsets.wait_start_provider, DRM_FENCE_PROVIDER_I915);
    assert_eq!(offsets.wait_start_gpu_role, DRM_GPU_ROLE_DISPLAY);
    assert_eq!(offsets.signal_provider, DRM_FENCE_PROVIDER_AMDGPU);
    assert_eq!(offsets.signal_gpu_role, DRM_GPU_ROLE_RENDER);
}

#[test]
fn parse_tracepoint_format_extracts_field_size_offset_and_signed_flag() {
    let format = parse_tracepoint_format(
        PathBuf::from("/tmp/format"),
        "\tfield:int pid;\toffset:24;\tsize:4;\tsigned:1;\n\
         \tfield:char rwbs[8];\toffset:32;\tsize:8;\tsigned:0;\n",
    );

    assert_eq!(
        format.fields.get("pid"),
        Some(&TracepointField {
            name: "pid".to_owned(),
            offset: 24,
            size: 4,
            signed: true,
            declaration: "field:int pid;\toffset:24;\tsize:4;\tsigned:1;".to_owned(),
        })
    );
    assert_eq!(
        format.fields.get("rwbs"),
        Some(&TracepointField {
            name: "rwbs".to_owned(),
            offset: 32,
            size: 8,
            signed: false,
            declaration: "field:char rwbs[8];\toffset:32;\tsize:8;\tsigned:0;".to_owned(),
        })
    );
}

#[test]
fn ebpf_block_io_uses_shared_fixed_metadata_offsets() {
    let source = include_str!("../../../../stutter-ebpf/src/block_io.rs");

    assert!(source.contains("BLOCK_RQ_DEV_OFFSET"));
    assert!(source.contains("BLOCK_RQ_SECTOR_OFFSET"));

    assert!(
        !source.contains("read_u32(&ctx, 8, &mut dev)"),
        "block_io eBPF must use BLOCK_RQ_DEV_OFFSET, not a raw dev offset"
    );
    assert!(
        !source.contains("read_u64(&ctx, 16, &mut sector)"),
        "block_io eBPF must use BLOCK_RQ_SECTOR_OFFSET, not a raw sector offset"
    );
}

#[test]
fn ebpf_block_io_complete_uses_loader_injected_nr_sector_offset() {
    let source = include_str!("../../../../stutter-ebpf/src/block_io.rs");

    assert!(source.contains("BLOCK_RQ_COMPLETE_NR_SECTOR_OFFSET"));
    assert!(
        !source.contains("read_u32(&ctx, 24, &mut nr_sector)"),
        "block_rq_complete must not hardcode nr_sector at offset 24"
    );
    assert!(
        source.contains("read_u32_or_zero(&ctx, nr_sector_offset)")
            || source.contains("ctx.read_at::<u32>(nr_sector_offset as usize)"),
        "block_rq_complete should read nr_sector through the complete tracepoint offset"
    );
}
