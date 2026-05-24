use aya_ebpf::helpers::{bpf_get_current_pid_tgid, bpf_get_smp_processor_id};
use stutter_common::{
    EVENT_KMS_FLIP, KMS_FLIP_EVENT_INTERVAL, KMS_FLIP_HAS_CRTC, KMS_FLIP_HAS_DONE_NS,
    KMS_FLIP_HAS_DURATION_NS, KMS_FLIP_HAS_REQUEST_NS, KMS_FLIP_HAS_SEQUENCE, KmsFlipEvent,
};

use crate::KmsFlipKey;

// -----------------------------------------------------------------------------
// KMS flip event emission
// -----------------------------------------------------------------------------

#[inline(always)]
pub(crate) fn emit_kms_flip_event(
    key: &KmsFlipKey,
    provider_and_event_kind: u32,
    has_sequence: bool,
    sequence: u64,
    has_start_ns: bool,
    start_ns: u64,
    done_ns: u64,
) {
    let duration_ns = if has_start_ns {
        done_ns.saturating_sub(start_ns)
    } else {
        0
    };

    let mut flags = KMS_FLIP_HAS_DONE_NS;
    if key.crtc_id != 0 {
        flags |= KMS_FLIP_HAS_CRTC;
    }
    if has_start_ns {
        flags |= KMS_FLIP_HAS_REQUEST_NS | KMS_FLIP_HAS_DURATION_NS;
    }
    if has_sequence {
        flags |= KMS_FLIP_HAS_SEQUENCE;
    }

    let provider = provider_and_event_kind >> 16;
    let completion_event_kind = provider_and_event_kind & 0xffff;
    let pid_tgid = bpf_get_current_pid_tgid();
    emit_ringbuf_event!(
        KmsFlipEvent,
        return,
        KmsFlipEvent {
            kind: EVENT_KMS_FLIP,
            event_kind: if has_start_ns {
                KMS_FLIP_EVENT_INTERVAL
            } else {
                completion_event_kind
            },
            provider,
            flags,
            pid: (pid_tgid >> 32) as u32,
            tid: (pid_tgid & 0xffff_ffff) as u32,
            cpu: bpf_get_smp_processor_id() as u32,
            card_minor: key.card_minor,
            crtc_id: key.crtc_id,
            pipe: key.pipe,
            sequence: if has_sequence { sequence } else { 0 },
            request_ns: if has_start_ns { start_ns } else { 0 },
            done_ns,
            duration_ns,
            timestamp_ns: done_ns,
        }
    );
}
