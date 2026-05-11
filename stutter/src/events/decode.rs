use stutter_common::{
    BlockIoEvent, CpuFreqEvent, EVENT_BLOCK_IO, EVENT_CPU_FREQ, EVENT_EXEC, EVENT_IRQ_LATENCY,
    EVENT_MIGRATION, EVENT_RUNNABLE_LATENCY, ExecEvent, IrqEvent, MigrationEvent, SchedulerEvent,
};

#[derive(Debug, Clone, Copy)]
pub enum DecodedEbpfEvent {
    Scheduler(SchedulerEvent),
    Irq(IrqEvent),
    Migration(MigrationEvent),
    CpuFreq(CpuFreqEvent),
    BlockIo(BlockIoEvent),
    Exec(ExecEvent),
    Unknown { kind: u32 },
    Short { kind: u32, len: usize },
}

pub fn decode_ebpf_event(kind: u32, bytes: &[u8]) -> DecodedEbpfEvent {
    match kind {
        EVENT_RUNNABLE_LATENCY => decode_or_short(bytes, kind, DecodedEbpfEvent::Scheduler),
        EVENT_IRQ_LATENCY => decode_or_short(bytes, kind, DecodedEbpfEvent::Irq),
        EVENT_MIGRATION => decode_or_short(bytes, kind, DecodedEbpfEvent::Migration),
        EVENT_CPU_FREQ => decode_or_short(bytes, kind, DecodedEbpfEvent::CpuFreq),
        EVENT_BLOCK_IO => decode_or_short(bytes, kind, DecodedEbpfEvent::BlockIo),
        EVENT_EXEC => decode_or_short(bytes, kind, DecodedEbpfEvent::Exec),
        other => DecodedEbpfEvent::Unknown { kind: other },
    }
}

pub fn read_event_unaligned<T: aya::Pod + Copy>(data: &[u8]) -> Option<T> {
    if data.len() < std::mem::size_of::<T>() {
        return None;
    }

    Some(unsafe { (data.as_ptr() as *const T).read_unaligned() })
}

fn decode_or_short<T>(
    bytes: &[u8],
    kind: u32,
    constructor: impl FnOnce(T) -> DecodedEbpfEvent,
) -> DecodedEbpfEvent
where
    T: aya::Pod + Copy,
{
    match read_event_unaligned::<T>(bytes) {
        Some(event) => constructor(event),
        None => DecodedEbpfEvent::Short {
            kind,
            len: bytes.len(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            switch_prev_state: 0,
        };

        let bytes = unsafe {
            std::slice::from_raw_parts(
                &event as *const SchedulerEvent as *const u8,
                std::mem::size_of::<SchedulerEvent>(),
            )
        };

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
    fn unknown_event_kind_reports_unknown() {
        let decoded = decode_ebpf_event(9999, &[1, 2, 3]);

        assert!(matches!(decoded, DecodedEbpfEvent::Unknown { kind: 9999 }));
    }
}
