#![allow(dead_code)]

use std::collections::BTreeMap;

use crate::{
    diagnosis::LiveDiagnosisEntry,
    ebpf_loader::DropCountersSnapshot,
    focus::FocusGroupKind,
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
    FocusChanged {
        elapsed_ms: u64,
        old_kind: Option<FocusGroupKind>,
        new_kind: FocusGroupKind,
        root_pids: Vec<u32>,
        member_pids: Vec<u32>,
        confidence: f32,
        score: f32,
        situation: crate::autotune::state::SituationKind,
        reasons: Vec<String>,
    },
    FocusCleared {
        elapsed_ms: u64,
        old_kind: Option<FocusGroupKind>,
        reason: String,
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
            Self::FocusChanged { .. } => "focus_changed",
            Self::FocusCleared { .. } => "focus_cleared",
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
            Self::FocusChanged { elapsed_ms, .. } => Some(*elapsed_ms),
            Self::FocusCleared { elapsed_ms, .. } => Some(*elapsed_ms),
            Self::Finished { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{autotune::state::SituationKind, focus::FocusGroupKind};

    #[test]
    fn focus_changed_event_reports_kind_and_elapsed_ms() {
        let event = MonitorEvent::FocusChanged {
            elapsed_ms: 1234,
            old_kind: Some(FocusGroupKind::Browser),
            new_kind: FocusGroupKind::Game,
            root_pids: vec![10],
            member_pids: vec![10, 11, 12],
            confidence: 0.75,
            score: 0.82,
            situation: SituationKind::GameFocused,
            reasons: vec!["test focus change".to_owned()],
        };

        assert_eq!(event.kind(), "focus_changed");
        assert_eq!(event.elapsed_ms(), Some(1234));
    }

    #[test]
    fn focus_cleared_event_reports_kind_and_elapsed_ms() {
        let event = MonitorEvent::FocusCleared {
            elapsed_ms: 5678,
            old_kind: Some(FocusGroupKind::Compile),
            reason: "no eligible focus group".to_owned(),
        };

        assert_eq!(event.kind(), "focus_cleared");
        assert_eq!(event.elapsed_ms(), Some(5678));
    }
}
