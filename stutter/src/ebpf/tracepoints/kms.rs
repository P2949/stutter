//! KMS tracepoint provider offset validation.

use crate::drm_tracepoints::{
    DrmTracepointField, DrmTracepointFormat, KmsTracepointAvailability, selected_request_format,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KmsProviderTracepointOffsets {
    pub(crate) request_crtc_offset: u32,
    pub(crate) request_pipe_offset: u32,
    pub(crate) done_crtc_offset: u32,
    pub(crate) done_pipe_offset: u32,
    pub(crate) done_sequence_offset: u32,
    pub(crate) done_sequence_size: u32,
    pub(crate) vblank_crtc_offset: u32,
    pub(crate) vblank_pipe_offset: u32,
    pub(crate) vblank_sequence_offset: u32,
    pub(crate) vblank_sequence_size: u32,
}

pub(crate) fn kms_provider_tracepoint_offsets(
    kms: &KmsTracepointAvailability,
) -> Option<KmsProviderTracepointOffsets> {
    if !kms.selected_provider_has_required_fields() {
        return None;
    }

    let request = selected_request_format(kms);
    let done = kms.pageflip_done.as_ref();
    let vblank = kms.vblank_event.as_ref();

    let request_crtc =
        request.and_then(|format| find_kms_field(format, &["crtc_id", "crtc", "crtc_index"]));
    let request_pipe = request.and_then(|format| find_kms_field(format, &["pipe"]));
    let done_crtc =
        done.and_then(|format| find_kms_field(format, &["crtc_id", "crtc", "crtc_index"]));
    let done_pipe = done.and_then(|format| find_kms_field(format, &["pipe"]));
    let done_sequence = done.and_then(|format| {
        find_kms_field(format, &["sequence", "seq", "vbl_count", "frame", "count"])
    });
    let vblank_crtc =
        vblank.and_then(|format| find_kms_field(format, &["crtc_id", "crtc", "crtc_index"]));
    let vblank_pipe = vblank.and_then(|format| find_kms_field(format, &["pipe"]));
    let vblank_sequence = vblank.and_then(|format| {
        find_kms_field(format, &["sequence", "seq", "vbl_count", "frame", "count"])
    });

    Some(KmsProviderTracepointOffsets {
        request_crtc_offset: request_crtc.map(|field| field.offset).unwrap_or(0),
        request_pipe_offset: request_pipe.map(|field| field.offset).unwrap_or(0),
        done_crtc_offset: done_crtc.map(|field| field.offset).unwrap_or(0),
        done_pipe_offset: done_pipe.map(|field| field.offset).unwrap_or(0),
        done_sequence_offset: done_sequence.map(|field| field.offset).unwrap_or(0),
        done_sequence_size: done_sequence.map(|field| field.size).unwrap_or(0),
        vblank_crtc_offset: vblank_crtc.map(|field| field.offset).unwrap_or(0),
        vblank_pipe_offset: vblank_pipe.map(|field| field.offset).unwrap_or(0),
        vblank_sequence_offset: vblank_sequence.map(|field| field.offset).unwrap_or(0),
        vblank_sequence_size: vblank_sequence.map(|field| field.size).unwrap_or(0),
    })
}

fn find_kms_field<'a>(
    format: &'a DrmTracepointFormat,
    names: &[&str],
) -> Option<&'a DrmTracepointField> {
    format.find_field(names)
}
