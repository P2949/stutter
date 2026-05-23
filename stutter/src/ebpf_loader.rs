#[cfg(test)]
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[cfg(test)]
use stutter_common::{
    DRM_FENCE_PROVIDER_AMDGPU, DRM_FENCE_PROVIDER_I915, DRM_GPU_ROLE_DISPLAY, DRM_GPU_ROLE_RENDER,
};

#[cfg(test)]
use crate::config::TARGET_PIDS_MAX;
pub(crate) use crate::ebpf::tracepoints::drm_fence::{
    drm_fence_probe_has_signal, drm_fence_probe_has_wait_interval, drm_fence_probe_supported,
};
pub use crate::ebpf::{
    load::load_and_attach,
    maps::ebpf_map_sizing_report,
    model::{BlockIoCorrelationBasis, DropCountersSnapshot, LoadedEbpf, NativeCgroupFilterStatus},
    preflight::{TracepointAvailability, tracepoint_preflight},
};
#[cfg(test)]
use crate::ebpf::{
    maps::{
        MAX_EVENTS_RINGBUF_BYTES, MAX_WAKEUP_DATA_ENTRIES, MIN_EVENTS_RINGBUF_BYTES,
        MIN_WAKEUP_DATA_ENTRIES, MemorySnapshot, WAKEUP_DATA_MAP_ENTRY_BUDGET_BYTES,
        map_sizing_for_config, map_sizing_from_memory, ring_buffer_size_from_budget,
        wakeup_data_entries_for_config,
    },
    memlock::memlock_limit_bytes_from_rlim,
    memory::parse_mem_available_bytes,
    model::MemlockPolicyReport,
    object::read_prebuilt_bpf_object,
    preflight::{sched_wakeup_new_coverage_status, validate_tracepoint_formats},
    tracepoint_format::{
        TracepointField, parse_tracepoint_field_offset, parse_tracepoint_format,
        validate_optional_tracepoint_format_at, validate_tracepoint_format,
        validate_tracepoint_format_named,
    },
    tracepoints::{
        block_io::validate_block_io_tracepoint_offsets, drm_fence::drm_fence_tracepoint_offsets,
        kms::kms_provider_tracepoint_offsets,
    },
};

#[cfg(test)]
#[path = "ebpf/tests/map_sizing.rs"]
mod map_sizing_tests;

#[cfg(test)]
#[path = "ebpf/tests/block_io_tracepoints.rs"]
mod block_io_tracepoint_validation_tests;

#[cfg(test)]
#[path = "ebpf/tests/sched_wakeup_new.rs"]
mod sched_wakeup_new_coverage_tests;

#[cfg(test)]
fn parse_tracepoint_offsets(format_content: &str) -> BTreeMap<String, TracepointField> {
    parse_tracepoint_format(PathBuf::from("tracepoint"), format_content).fields
}

#[cfg(test)]
fn find_request_key_offset(offsets: &BTreeMap<String, TracepointField>) -> Option<u32> {
    for name in ["rq", "req", "request"] {
        if let Some(field) = offsets.get(name)
            && field.offset >= 8
            && field.offset % 8 == 0
            && field.size == 8
        {
            return Some(field.offset);
        }
    }

    None
}

#[cfg(test)]
fn matching_request_key_offset(
    issue_offsets: &BTreeMap<String, TracepointField>,
    complete_offsets: &BTreeMap<String, TracepointField>,
) -> Option<u32> {
    let issue_key_offset = find_request_key_offset(issue_offsets);
    let complete_key_offset = find_request_key_offset(complete_offsets);

    if issue_key_offset == complete_key_offset {
        issue_key_offset
    } else {
        None
    }
}

#[cfg(test)]
#[path = "ebpf/tests/loader.rs"]
mod tests;
