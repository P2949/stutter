//! Pure eBPF loader configuration planning.

use aya::EbpfLoader;

use crate::{
    drm_tracepoints::KmsTracepointProvider,
    ebpf::{
        model::{BlockIoCorrelationBasis, EbpfMapSizing},
        preflight::TracepointAvailability,
        tracepoints::{
            drm_fence::{DrmFenceTracepointOffsets, drm_fence_tracepoint_offsets},
            kms::{KmsProviderTracepointOffsets, kms_provider_tracepoint_offsets},
        },
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MapMaxEntriesOverride {
    pub(crate) map_name: &'static str,
    pub(crate) entries: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GlobalOverride {
    pub(crate) name: &'static str,
    pub(crate) value: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LoaderPlan {
    pub(crate) map_max_entries: Vec<MapMaxEntriesOverride>,
    pub(crate) global_overrides: Vec<GlobalOverride>,
    pub(crate) block_io_correlation_basis: BlockIoCorrelationBasis,
    pub(crate) drm_fence_offsets: Option<DrmFenceTracepointOffsets>,
}

#[cfg(test)]
impl LoaderPlan {
    pub(crate) fn global_value(&self, name: &str) -> Option<u32> {
        self.global_overrides
            .iter()
            .find(|override_| override_.name == name)
            .map(|override_| override_.value)
    }

    pub(crate) fn map_entries(&self, name: &str) -> Option<u32> {
        self.map_max_entries
            .iter()
            .find(|override_| override_.map_name == name)
            .map(|override_| override_.entries)
    }
}

pub(crate) fn build_loader_plan(
    tracepoints: &TracepointAvailability,
    map_sizing: EbpfMapSizing,
) -> LoaderPlan {
    let mut plan = LoaderPlan {
        map_max_entries: vec![
            MapMaxEntriesOverride {
                map_name: "EVENTS",
                entries: map_sizing.events_ringbuf_bytes,
            },
            MapMaxEntriesOverride {
                map_name: "WAKEUP_DATA",
                entries: map_sizing.wakeup_data_entries,
            },
            MapMaxEntriesOverride {
                map_name: "WAKEUP_CONSUMED",
                entries: map_sizing.wakeup_data_entries,
            },
        ],
        global_overrides: Vec::new(),
        block_io_correlation_basis: block_io_correlation_basis(tracepoints),
        drm_fence_offsets: tracepoints
            .drm_fence
            .as_ref()
            .and_then(drm_fence_tracepoint_offsets),
    };

    append_block_io_overrides(&mut plan, tracepoints);

    if let Some(kms_offsets) = kms_provider_tracepoint_offsets(&tracepoints.kms) {
        append_kms_overrides(&mut plan, tracepoints.kms.provider, kms_offsets);
    }

    if let Some(offsets) = plan.drm_fence_offsets {
        append_drm_fence_overrides(&mut plan, offsets);
    }

    plan
}

pub(crate) fn apply_loader_plan<'a>(loader: &mut EbpfLoader<'a>, plan: &'a LoaderPlan) {
    for override_ in &plan.map_max_entries {
        loader.map_max_entries(override_.map_name, override_.entries);
    }

    for override_ in &plan.global_overrides {
        loader.override_global(override_.name, &override_.value, true);
    }
}

fn block_io_correlation_basis(tracepoints: &TracepointAvailability) -> BlockIoCorrelationBasis {
    if !tracepoints.block_rq {
        BlockIoCorrelationBasis::Disabled
    } else if tracepoints.block_rq_key_offset.is_some() {
        BlockIoCorrelationBasis::RequestPointer
    } else {
        BlockIoCorrelationBasis::DevSector
    }
}

fn push_global(plan: &mut LoaderPlan, name: &'static str, value: u32) {
    plan.global_overrides.push(GlobalOverride { name, value });
}

fn append_block_io_overrides(plan: &mut LoaderPlan, tracepoints: &TracepointAvailability) {
    if !tracepoints.block_rq {
        return;
    }

    if let Some(offset) = tracepoints.block_rq_key_offset {
        push_global(plan, "BLOCK_RQ_KEY_OFFSET", offset);
    }
    if let Some(offset) = tracepoints.block_rq_issue_nr_sector_offset {
        push_global(plan, "BLOCK_RQ_ISSUE_NR_SECTOR_OFFSET", offset);
    }
    if let Some(offset) = tracepoints.block_rq_issue_rwbs_offset {
        push_global(plan, "BLOCK_RQ_ISSUE_RWBS_OFFSET", offset);
    }
    if let Some(offset) = tracepoints.block_rq_complete_nr_sector_offset {
        push_global(plan, "BLOCK_RQ_COMPLETE_NR_SECTOR_OFFSET", offset);
    }
    if let Some(offset) = tracepoints.block_rq_complete_rwbs_offset {
        push_global(plan, "BLOCK_RQ_COMPLETE_RWBS_OFFSET", offset);
    }
}

fn append_kms_overrides(
    plan: &mut LoaderPlan,
    provider: KmsTracepointProvider,
    offsets: KmsProviderTracepointOffsets,
) {
    match provider {
        KmsTracepointProvider::I915 => {
            push_global(
                plan,
                "I915_FLIP_REQUEST_CRTC_OFFSET",
                offsets.request_crtc_offset,
            );
            push_global(
                plan,
                "I915_FLIP_REQUEST_PIPE_OFFSET",
                offsets.request_pipe_offset,
            );
            push_global(plan, "I915_FLIP_DONE_CRTC_OFFSET", offsets.done_crtc_offset);
            push_global(plan, "I915_FLIP_DONE_PIPE_OFFSET", offsets.done_pipe_offset);
            push_global(
                plan,
                "I915_FLIP_DONE_SEQUENCE_OFFSET",
                offsets.done_sequence_offset,
            );
            push_global(
                plan,
                "I915_FLIP_DONE_SEQUENCE_SIZE",
                offsets.done_sequence_size,
            );
        }
        KmsTracepointProvider::GenericDrm => {
            push_global(
                plan,
                "DRM_FLIP_REQUEST_CRTC_OFFSET",
                offsets.request_crtc_offset,
            );
            push_global(
                plan,
                "DRM_FLIP_REQUEST_PIPE_OFFSET",
                offsets.request_pipe_offset,
            );
            push_global(plan, "DRM_FLIP_DONE_CRTC_OFFSET", offsets.done_crtc_offset);
            push_global(plan, "DRM_FLIP_DONE_PIPE_OFFSET", offsets.done_pipe_offset);
            push_global(
                plan,
                "DRM_FLIP_DONE_SEQUENCE_OFFSET",
                offsets.done_sequence_offset,
            );
            push_global(
                plan,
                "DRM_FLIP_DONE_SEQUENCE_SIZE",
                offsets.done_sequence_size,
            );
            push_global(plan, "DRM_VBLANK_CRTC_OFFSET", offsets.vblank_crtc_offset);
            push_global(plan, "DRM_VBLANK_PIPE_OFFSET", offsets.vblank_pipe_offset);
            push_global(
                plan,
                "DRM_VBLANK_SEQUENCE_OFFSET",
                offsets.vblank_sequence_offset,
            );
            push_global(
                plan,
                "DRM_VBLANK_SEQUENCE_SIZE",
                offsets.vblank_sequence_size,
            );
        }
        KmsTracepointProvider::Amdgpu => {
            push_global(
                plan,
                "AMDGPU_FLIP_REQUEST_CRTC_OFFSET",
                offsets.request_crtc_offset,
            );
            push_global(
                plan,
                "AMDGPU_FLIP_REQUEST_PIPE_OFFSET",
                offsets.request_pipe_offset,
            );
            push_global(
                plan,
                "AMDGPU_FLIP_DONE_CRTC_OFFSET",
                offsets.done_crtc_offset,
            );
            push_global(
                plan,
                "AMDGPU_FLIP_DONE_PIPE_OFFSET",
                offsets.done_pipe_offset,
            );
            push_global(
                plan,
                "AMDGPU_FLIP_DONE_SEQUENCE_OFFSET",
                offsets.done_sequence_offset,
            );
            push_global(
                plan,
                "AMDGPU_FLIP_DONE_SEQUENCE_SIZE",
                offsets.done_sequence_size,
            );
            push_global(
                plan,
                "AMDGPU_VBLANK_CRTC_OFFSET",
                offsets.vblank_crtc_offset,
            );
            push_global(
                plan,
                "AMDGPU_VBLANK_PIPE_OFFSET",
                offsets.vblank_pipe_offset,
            );
            push_global(
                plan,
                "AMDGPU_VBLANK_SEQUENCE_OFFSET",
                offsets.vblank_sequence_offset,
            );
            push_global(
                plan,
                "AMDGPU_VBLANK_SEQUENCE_SIZE",
                offsets.vblank_sequence_size,
            );
        }
        KmsTracepointProvider::Mixed | KmsTracepointProvider::Unavailable => {}
    }
}

fn append_drm_fence_overrides(plan: &mut LoaderPlan, offsets: DrmFenceTracepointOffsets) {
    push_global(
        plan,
        "DRM_FENCE_WAIT_START_CONTEXT_OFFSET",
        offsets.wait_start_context_offset,
    );
    push_global(
        plan,
        "DRM_FENCE_WAIT_START_SEQNO_OFFSET",
        offsets.wait_start_seqno_offset,
    );
    push_global(
        plan,
        "DRM_FENCE_WAIT_START_TIMELINE_OFFSET",
        offsets.wait_start_timeline_offset,
    );
    push_global(
        plan,
        "DRM_FENCE_WAIT_DONE_CONTEXT_OFFSET",
        offsets.wait_done_context_offset,
    );
    push_global(
        plan,
        "DRM_FENCE_WAIT_DONE_SEQNO_OFFSET",
        offsets.wait_done_seqno_offset,
    );
    push_global(
        plan,
        "DRM_FENCE_WAIT_DONE_TIMELINE_OFFSET",
        offsets.wait_done_timeline_offset,
    );
    push_global(
        plan,
        "DRM_FENCE_SIGNAL_CONTEXT_OFFSET",
        offsets.signal_context_offset,
    );
    push_global(
        plan,
        "DRM_FENCE_SIGNAL_SEQNO_OFFSET",
        offsets.signal_seqno_offset,
    );
    push_global(
        plan,
        "DRM_FENCE_SIGNAL_TIMELINE_OFFSET",
        offsets.signal_timeline_offset,
    );
    push_global(
        plan,
        "DRM_FENCE_WAIT_START_PROVIDER",
        offsets.wait_start_provider,
    );
    push_global(
        plan,
        "DRM_FENCE_WAIT_START_GPU_ROLE",
        offsets.wait_start_gpu_role,
    );
    push_global(
        plan,
        "DRM_FENCE_WAIT_DONE_PROVIDER",
        offsets.wait_done_provider,
    );
    push_global(
        plan,
        "DRM_FENCE_WAIT_DONE_GPU_ROLE",
        offsets.wait_done_gpu_role,
    );
    push_global(plan, "DRM_FENCE_SIGNAL_PROVIDER", offsets.signal_provider);
    push_global(plan, "DRM_FENCE_SIGNAL_GPU_ROLE", offsets.signal_gpu_role);
}
