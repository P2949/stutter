use stutter_common::{
    BlockIoEvent, CpuFreqEvent, DrmFenceEvent, EVENT_BLOCK_IO, EVENT_CPU_FREQ, EVENT_DRM_FENCE,
    EVENT_EXEC, EVENT_IRQ_LATENCY, EVENT_KMS_FLIP, EVENT_MIGRATION, EVENT_RUNNABLE_LATENCY,
    EVENT_STAT_WAIT, ExecEvent, IrqEvent, KmsFlipEvent, MigrationEvent, SchedulerEvent,
    StatWaitEvent,
};

#[derive(Debug, Clone, Copy)]
pub enum DecodedEbpfEvent {
    Scheduler(SchedulerEvent),
    Irq(IrqEvent),
    Migration(MigrationEvent),
    CpuFreq(CpuFreqEvent),
    StatWait(StatWaitEvent),
    BlockIo(BlockIoEvent),
    Exec(ExecEvent),
    KmsFlip(KmsFlipEvent),
    DrmFence(DrmFenceEvent),
    Unknown { kind: u32 },
    Short { kind: u32, len: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventDecodeLayout {
    pub size: usize,
    pub align: usize,
}

impl EventDecodeLayout {
    fn of<T>() -> Self {
        Self {
            size: std::mem::size_of::<T>(),
            align: std::mem::align_of::<T>(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventDecodeError {
    Short {
        actual_len: usize,
        layout: EventDecodeLayout,
    },
}

pub fn decode_ebpf_event(kind: u32, bytes: &[u8]) -> DecodedEbpfEvent {
    match kind {
        EVENT_RUNNABLE_LATENCY => decode_or_short(bytes, kind, DecodedEbpfEvent::Scheduler),
        EVENT_IRQ_LATENCY => decode_or_short(bytes, kind, DecodedEbpfEvent::Irq),
        EVENT_MIGRATION => decode_or_short(bytes, kind, DecodedEbpfEvent::Migration),
        EVENT_CPU_FREQ => decode_or_short(bytes, kind, DecodedEbpfEvent::CpuFreq),
        EVENT_STAT_WAIT => decode_or_short(bytes, kind, DecodedEbpfEvent::StatWait),
        EVENT_BLOCK_IO => decode_or_short(bytes, kind, DecodedEbpfEvent::BlockIo),
        EVENT_EXEC => decode_or_short(bytes, kind, DecodedEbpfEvent::Exec),
        EVENT_KMS_FLIP => decode_or_short(bytes, kind, DecodedEbpfEvent::KmsFlip),
        EVENT_DRM_FENCE => decode_or_short(bytes, kind, DecodedEbpfEvent::DrmFence),
        other => DecodedEbpfEvent::Unknown { kind: other },
    }
}

pub fn read_u32_unaligned(data: &[u8]) -> Option<u32> {
    if data.len() < std::mem::size_of::<u32>() {
        return None;
    }
    // SAFETY: data length is checked to be at least 4 bytes. read_unaligned handles alignment.
    Some(unsafe { (data.as_ptr() as *const u32).read_unaligned() })
}

pub fn read_event_unaligned<T: aya::Pod + Copy>(data: &[u8]) -> Option<T> {
    read_event_unaligned_checked(data).ok()
}

pub fn read_event_unaligned_checked<T: aya::Pod + Copy>(
    data: &[u8],
) -> Result<T, EventDecodeError> {
    let layout = EventDecodeLayout::of::<T>();
    if data.len() < layout.size {
        return Err(EventDecodeError::Short {
            actual_len: data.len(),
            layout,
        });
    }
    debug_assert!(layout.align.is_power_of_two());

    // SAFETY: length is checked above, T is Pod + Copy, and read_unaligned is
    // used because ring-buffer bytes may not be naturally aligned for T.
    Ok(unsafe { (data.as_ptr() as *const T).read_unaligned() })
}

fn decode_or_short<T>(
    bytes: &[u8],
    kind: u32,
    constructor: impl FnOnce(T) -> DecodedEbpfEvent,
) -> DecodedEbpfEvent
where
    T: aya::Pod + Copy,
{
    match read_event_unaligned_checked::<T>(bytes) {
        Ok(event) => constructor(event),
        Err(EventDecodeError::Short { .. }) => DecodedEbpfEvent::Short {
            kind,
            len: bytes.len(),
        },
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn as_bytes<T: aya::Pod>(val: &T) -> &[u8] {
        // SAFETY: val is a Pod, so it is safe to view as bytes.
        unsafe {
            std::slice::from_raw_parts((val as *const T).cast::<u8>(), std::mem::size_of::<T>())
        }
    }

    #[test]
    fn test_unaligned_event_decoding() {
        let event = SchedulerEvent {
            kind: EVENT_RUNNABLE_LATENCY,
            tid: 123,
            cpu: 1,
            wakeup_target_cpu: 1,
            prio: 120,
            waker_tid: 0,
            target_pending_wakeups: 0,
            observed_runnable_depth: 0,
            maj_flt: 0,
            min_flt: 0,
            wakeup_ns: 2000,
            switch_ns: 3000,
            latency_ns: 1000,
            comm: [0; 16],
            switch_prev_pid: 0,
            _pad0: 0,
            switch_prev_state: 0,
        };

        let bytes = as_bytes(&event);

        let mut misaligned = vec![0u8];
        misaligned.extend_from_slice(bytes);

        let decoded = read_event_unaligned::<SchedulerEvent>(&misaligned[1..]).unwrap();
        assert_eq!(decoded.kind, EVENT_RUNNABLE_LATENCY);
        assert_eq!(decoded.tid, 123);
    }

    #[test]
    fn short_scheduler_event_reports_short() {
        let decoded = decode_ebpf_event(EVENT_RUNNABLE_LATENCY, &[1, 2, 3]);

        assert!(matches!(
            decoded,
            DecodedEbpfEvent::Short {
                kind: EVENT_RUNNABLE_LATENCY,
                len: 3
            }
        ));
    }

    #[test]
    fn checked_decode_reports_required_layout_for_short_input() {
        let err = read_event_unaligned_checked::<SchedulerEvent>(&[1, 2, 3]).unwrap_err();

        assert_eq!(
            err,
            EventDecodeError::Short {
                actual_len: 3,
                layout: EventDecodeLayout {
                    size: std::mem::size_of::<SchedulerEvent>(),
                    align: std::mem::align_of::<SchedulerEvent>(),
                },
            }
        );
    }

    #[test]
    fn unknown_event_kind_reports_unknown() {
        let decoded = decode_ebpf_event(9999, &[1, 2, 3]);

        assert!(matches!(decoded, DecodedEbpfEvent::Unknown { kind: 9999 }));
    }

    #[test]
    fn decode_stat_wait_event_reports_stat_wait() {
        let event = StatWaitEvent {
            kind: EVENT_STAT_WAIT,
            tid: 123,
            delay_ns: 1000,
        };
        let bytes = as_bytes(&event);
        let decoded = decode_ebpf_event(EVENT_STAT_WAIT, bytes);
        assert!(matches!(decoded, DecodedEbpfEvent::StatWait(_)));
        if let DecodedEbpfEvent::StatWait(d) = decoded {
            assert_eq!(d.kind, EVENT_STAT_WAIT);
        }
    }

    #[test]
    fn decode_kms_flip_event_reports_kms_flip() {
        let event = KmsFlipEvent {
            kind: EVENT_KMS_FLIP,
            event_kind: 1,
            provider: 1,
            flags: 0,
            pid: 123,
            tid: 123,
            cpu: 1,
            card_minor: 0,
            crtc_id: 1,
            pipe: 0,
            sequence: 1,
            request_ns: 1000,
            done_ns: 2000,
            duration_ns: 1000,
            timestamp_ns: 3000,
        };
        let bytes = as_bytes(&event);
        let decoded = decode_ebpf_event(EVENT_KMS_FLIP, bytes);
        assert!(matches!(decoded, DecodedEbpfEvent::KmsFlip(_)));
        if let DecodedEbpfEvent::KmsFlip(d) = decoded {
            assert_eq!(d.kind, EVENT_KMS_FLIP);
        }
    }

    #[test]
    fn decode_drm_fence_event_reports_drm_fence() {
        let event = DrmFenceEvent {
            kind: EVENT_DRM_FENCE,
            event_kind: 1,
            provider: 1,
            flags: 0,
            pid: 123,
            tid: 123,
            cpu: 1,
            driver_id: 1,
            gpu_role: 0,
            _pad0: 0,
            context: 1,
            seqno: 1,
            timeline_hash: 0,
            wait_start_ns: 1000,
            wait_done_ns: 2000,
            signal_ns: 1500,
            duration_ns: 1000,
            timestamp_ns: 3000,
        };
        let bytes = as_bytes(&event);
        let decoded = decode_ebpf_event(EVENT_DRM_FENCE, bytes);
        assert!(matches!(decoded, DecodedEbpfEvent::DrmFence(_)));
        if let DecodedEbpfEvent::DrmFence(d) = decoded {
            assert_eq!(d.kind, EVENT_DRM_FENCE);
        }
    }

    #[test]
    fn short_new_events_report_short() {
        let decoded = decode_ebpf_event(EVENT_STAT_WAIT, &[1, 2, 3]);
        assert!(matches!(
            decoded,
            DecodedEbpfEvent::Short {
                kind: EVENT_STAT_WAIT,
                len: 3
            }
        ));

        let decoded = decode_ebpf_event(EVENT_KMS_FLIP, &[1, 2, 3]);
        assert!(matches!(
            decoded,
            DecodedEbpfEvent::Short {
                kind: EVENT_KMS_FLIP,
                len: 3
            }
        ));

        let decoded = decode_ebpf_event(EVENT_DRM_FENCE, &[1, 2, 3]);
        assert!(matches!(
            decoded,
            DecodedEbpfEvent::Short {
                kind: EVENT_DRM_FENCE,
                len: 3
            }
        ));
    }
}
