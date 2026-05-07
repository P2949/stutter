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
