//! Userspace event shapes converted from raw `stutter-common` ABI records.
use stutter_common::IrqEvent as RawIrqEvent;

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
            _pad0: 0,
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
