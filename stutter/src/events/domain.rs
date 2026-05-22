//! Userspace event shapes converted from raw `stutter-common` ABI records.
#![allow(dead_code)] // Transitional event boundary: raw decoders migrate to these wrappers path-by-path.
// Exit: remove this marker once the described migration is complete and the local allow is no longer needed.

use stutter_common::{
    BlockIoEvent as RawBlockIoEvent, CpuFreqEvent as RawCpuFreqEvent, IrqEvent as RawIrqEvent,
    MigrationEvent as RawMigrationEvent, SchedulerEvent as RawSchedulerEvent,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SchedulerLatencyEvent {
    pub tid: u32,
    pub cpu: u32,
    pub wakeup_target_cpu: u32,
    pub prio: i32,
    pub latency_ns: u64,
    pub wakeup_ns: u64,
    pub switch_ns: u64,
    pub waker_tid: u32,
    pub observed_runnable_depth: u32,
    pub target_pending_wakeups: u32,
}

impl SchedulerLatencyEvent {
    pub(crate) fn from_raw(raw: RawSchedulerEvent) -> Self {
        Self {
            tid: raw.tid,
            cpu: raw.cpu,
            wakeup_target_cpu: raw.wakeup_target_cpu,
            prio: raw.prio,
            latency_ns: raw.latency_ns,
            wakeup_ns: raw.wakeup_ns,
            switch_ns: raw.switch_ns,
            waker_tid: raw.waker_tid,
            observed_runnable_depth: raw.observed_runnable_depth,
            target_pending_wakeups: raw.target_pending_wakeups,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FrameEvent {
    pub elapsed_ms: u64,
    pub frame_time_ns: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GpuSampleEvent {
    pub device: String,
    pub utilization_percent: Option<u8>,
    pub frequency_mhz: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IrqEvent {
    pub irq: u32,
    pub cpu: u32,
    pub enter_ns: u64,
    pub exit_ns: u64,
    pub duration_ns: u64,
}

impl IrqEvent {
    pub(crate) fn from_raw(raw: RawIrqEvent) -> Self {
        Self {
            irq: raw.irq,
            cpu: raw.cpu,
            enter_ns: raw.enter_ns,
            exit_ns: raw.exit_ns,
            duration_ns: raw.duration_ns,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IoEvent {
    pub tid: u32,
    pub dev: u32,
    pub nr_sector: u32,
    pub sector: u64,
    pub duration_ns: u64,
    pub timestamp_ns: u64,
    pub rwbs: String,
}

impl IoEvent {
    pub(crate) fn from_raw(raw: RawBlockIoEvent) -> Self {
        Self {
            tid: raw.tid,
            dev: raw.dev,
            nr_sector: raw.nr_sector,
            sector: raw.sector,
            duration_ns: raw.duration_ns,
            timestamp_ns: raw.timestamp_ns,
            rwbs: String::from_utf8_lossy(&raw.rwbs)
                .trim_matches(char::from(0))
                .to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CpuFrequencyEvent {
    pub cpu: u32,
    pub freq_khz: u32,
    pub timestamp_ns: u64,
}

impl CpuFrequencyEvent {
    pub(crate) fn from_raw(raw: RawCpuFreqEvent) -> Self {
        Self {
            cpu: raw.cpu,
            freq_khz: raw.state,
            timestamp_ns: raw.timestamp_ns,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MigrationEvent {
    pub tid: u32,
    pub from_cpu: u32,
    pub to_cpu: u32,
    pub timestamp_ns: u64,
}

impl MigrationEvent {
    pub(crate) fn from_raw(raw: RawMigrationEvent) -> Self {
        Self {
            tid: raw.tid,
            from_cpu: raw.from_cpu,
            to_cpu: raw.to_cpu,
            timestamp_ns: raw.timestamp_ns,
        }
    }
}

#[cfg(test)]
mod tests {
    use stutter_common::{EVENT_IRQ_LATENCY, IrqEvent as RawIrqEvent};

    use super::*;

    #[test]
    fn irq_domain_event_drops_only_abi_discriminator() {
        let raw = RawIrqEvent {
            kind: EVENT_IRQ_LATENCY,
            irq: 9,
            cpu: 2,
            enter_ns: 10,
            exit_ns: 40,
            duration_ns: 30,
        };

        let event = IrqEvent::from_raw(raw);

        assert_eq!(event.irq, 9);
        assert_eq!(event.cpu, 2);
        assert_eq!(event.duration_ns, 30);
    }
}
