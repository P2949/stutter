#![allow(dead_code)]

use std::mem::size_of;

use stutter_common::{
    BlockIoEvent, CpuFreqEvent, EVENT_BLOCK_IO, EVENT_CPU_FREQ, EVENT_EXEC, EVENT_IRQ_LATENCY,
    EVENT_MIGRATION, EVENT_RUNNABLE_LATENCY, EVENT_STAT_WAIT, ExecEvent, IrqEvent, MigrationEvent,
    SchedulerEvent, StatWaitEvent,
};

#[derive(Clone, Copy)]
pub enum KernelEvent {
    Runnable(SchedulerEvent),
    Irq(IrqEvent),
    Migration(MigrationEvent),
    CpuFreq(CpuFreqEvent),
    StatWait(StatWaitEvent),
    BlockIo(BlockIoEvent),
    Exec(ExecEvent),
}

impl std::fmt::Debug for KernelEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Runnable(_) => f.write_str("KernelEvent::Runnable(..)"),
            Self::Irq(_) => f.write_str("KernelEvent::Irq(..)"),
            Self::Migration(_) => f.write_str("KernelEvent::Migration(..)"),
            Self::CpuFreq(_) => f.write_str("KernelEvent::CpuFreq(..)"),
            Self::StatWait(_) => f.write_str("KernelEvent::StatWait(..)"),
            Self::BlockIo(_) => f.write_str("KernelEvent::BlockIo(..)"),
            Self::Exec(_) => f.write_str("KernelEvent::Exec(..)"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KernelEventDecodeError {
    TooShort {
        kind: Option<u32>,
        len: usize,
    },
    UnknownKind(u32),
    SizeMismatch {
        kind: u32,
        expected: usize,
        actual: usize,
    },
}

pub struct KernelEventDecoder;

impl KernelEventDecoder {
    pub fn decode(bytes: &[u8]) -> Result<KernelEvent, KernelEventDecodeError> {
        let kind = read_kind(bytes)?;
        match kind {
            EVENT_RUNNABLE_LATENCY => {
                decode_exact::<SchedulerEvent>(bytes, kind).map(KernelEvent::Runnable)
            }
            EVENT_IRQ_LATENCY => decode_exact::<IrqEvent>(bytes, kind).map(KernelEvent::Irq),
            EVENT_MIGRATION => {
                decode_exact::<MigrationEvent>(bytes, kind).map(KernelEvent::Migration)
            }
            EVENT_CPU_FREQ => decode_exact::<CpuFreqEvent>(bytes, kind).map(KernelEvent::CpuFreq),
            EVENT_STAT_WAIT => {
                decode_exact::<StatWaitEvent>(bytes, kind).map(KernelEvent::StatWait)
            }
            EVENT_BLOCK_IO => decode_exact::<BlockIoEvent>(bytes, kind).map(KernelEvent::BlockIo),
            EVENT_EXEC => decode_exact::<ExecEvent>(bytes, kind).map(KernelEvent::Exec),
            other => Err(KernelEventDecodeError::UnknownKind(other)),
        }
    }
}

fn read_kind(bytes: &[u8]) -> Result<u32, KernelEventDecodeError> {
    if bytes.len() < size_of::<u32>() {
        return Err(KernelEventDecodeError::TooShort {
            kind: None,
            len: bytes.len(),
        });
    }

    let mut kind_bytes = [0u8; size_of::<u32>()];
    kind_bytes.copy_from_slice(&bytes[..size_of::<u32>()]);
    Ok(u32::from_ne_bytes(kind_bytes))
}

fn decode_exact<T: Copy>(bytes: &[u8], kind: u32) -> Result<T, KernelEventDecodeError> {
    let expected = size_of::<T>();
    if bytes.len() != expected {
        return Err(KernelEventDecodeError::SizeMismatch {
            kind,
            expected,
            actual: bytes.len(),
        });
    }

    let value = unsafe { std::ptr::read_unaligned(bytes.as_ptr().cast::<T>()) };
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_too_short_event() {
        assert_eq!(
            KernelEventDecoder::decode(&[1, 2]).unwrap_err(),
            KernelEventDecodeError::TooShort { kind: None, len: 2 }
        );
    }

    #[test]
    fn rejects_unknown_kind() {
        assert_eq!(
            KernelEventDecoder::decode(&999u32.to_ne_bytes()).unwrap_err(),
            KernelEventDecodeError::UnknownKind(999)
        );
    }

    #[test]
    fn rejects_size_mismatch_for_known_kind() {
        assert_eq!(
            KernelEventDecoder::decode(&EVENT_RUNNABLE_LATENCY.to_ne_bytes()).unwrap_err(),
            KernelEventDecodeError::SizeMismatch {
                kind: EVENT_RUNNABLE_LATENCY,
                expected: size_of::<SchedulerEvent>(),
                actual: size_of::<u32>(),
            }
        );
    }
}
