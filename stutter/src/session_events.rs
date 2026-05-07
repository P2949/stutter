#![allow(dead_code)]

use std::collections::BTreeMap;

use crate::{
    diagnosis::LiveDiagnosisEntry,
    ebpf_loader::DropCountersSnapshot,
    process_tree::TaskInfo,
    recorder::{BlockIoRecord, FrameEvent, GpuSample, IntervalRecord, IrqEventRecord, SpikeEvent},
};

#[derive(Clone, Debug)]
pub enum MonitorEvent {
    TargetSnapshot {
        elapsed_ms: u64,
        active_targets: BTreeMap<u32, TaskInfo>,
        removed_targets: Vec<u32>,
    },
    Interval {
        elapsed_ms: u64,
        records: Vec<IntervalRecord>,
        drop_counters: DropCountersSnapshot,
    },
    Spike {
        event: Box<SpikeEvent>,
    },
    Frame {
        event: Box<FrameEvent>,
    },
    GpuSample {
        sample: Box<GpuSample>,
    },
    IrqEvent {
        event: Box<IrqEventRecord>,
    },
    IoEvent {
        event: Box<BlockIoRecord>,
    },
    LiveDiagnosis {
        entry: Box<LiveDiagnosisEntry>,
    },
    DataQualityWarning {
        message: String,
    },
    Finished {
        reason: String,
    },
}

impl MonitorEvent {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::TargetSnapshot { .. } => "target_snapshot",
            Self::Interval { .. } => "interval",
            Self::Spike { .. } => "spike",
            Self::Frame { .. } => "frame",
            Self::GpuSample { .. } => "gpu_sample",
            Self::IrqEvent { .. } => "irq_event",
            Self::IoEvent { .. } => "io_event",
            Self::LiveDiagnosis { .. } => "live_diagnosis",
            Self::DataQualityWarning { .. } => "data_quality_warning",
            Self::Finished { .. } => "finished",
        }
    }

    pub fn elapsed_ms(&self) -> Option<u64> {
        match self {
            Self::TargetSnapshot { elapsed_ms, .. } => Some(*elapsed_ms),
            Self::Interval { elapsed_ms, .. } => Some(*elapsed_ms),
            Self::Spike { event } => event.elapsed_ms,
            Self::Frame { event } => Some(event.elapsed_ms),
            Self::GpuSample { sample } => Some(sample.elapsed_ms),
            Self::IrqEvent { event } => event.elapsed_ms,
            Self::IoEvent { event } => Some(event.elapsed_ms),
            Self::LiveDiagnosis { entry } => Some(entry.elapsed_ms),
            Self::DataQualityWarning { .. } => None,
            Self::Finished { .. } => None,
        }
    }
}
