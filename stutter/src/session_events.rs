use std::collections::BTreeMap;

use stutter_common::SchedulerEvent;

use crate::{
    alert::AlertPayload,
    diagnosis::LiveDiagnosisEntry,
    ebpf_loader::DropCountersSnapshot,
    focus::FocusGroupKind,
    process_tree::TaskInfo,
    recorder::{
        BlockIoRecord, CpuFreqRecord, FocusEvent, ForegroundEvent, FrameEvent, GpuSample,
        IntervalRecord, IrqEventRecord, MigrationEventRecord, SpikeEvent,
    },
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
    SchedulerSample {
        event: Box<SchedulerEvent>,
        comm: String,
        label: &'static str,
    },
    Spike {
        event: Box<SpikeEvent>,
    },
    Alert {
        payload: Box<AlertPayload>,
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
    MigrationEvent {
        event: Box<MigrationEventRecord>,
    },
    CpuFreqSample {
        event: Box<CpuFreqRecord>,
    },
    ForegroundEvent {
        event: Box<ForegroundEvent>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonitorEventDeliveryClass {
    Reliable,
    Conflated,
    Droppable,
}

impl MonitorEvent {
    pub fn delivery_class(&self) -> MonitorEventDeliveryClass {
        match self {
            Self::Finished { .. }
            | Self::DataQualityWarning { .. }
            | Self::TargetSnapshot { .. }
            | Self::FocusChanged { .. }
            | Self::FocusCleared { .. }
            | Self::ForegroundEvent { .. } => MonitorEventDeliveryClass::Reliable,
            Self::Interval { .. } => MonitorEventDeliveryClass::Conflated,
            Self::SchedulerSample { .. }
            | Self::Spike { .. }
            | Self::Alert { .. }
            | Self::Frame { .. }
            | Self::GpuSample { .. }
            | Self::IrqEvent { .. }
            | Self::IoEvent { .. }
            | Self::MigrationEvent { .. }
            | Self::CpuFreqSample { .. }
            | Self::LiveDiagnosis { .. } => MonitorEventDeliveryClass::Droppable,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::TargetSnapshot { .. } => "target_snapshot",
            Self::Interval { .. } => "interval",
            Self::SchedulerSample { .. } => "scheduler_sample",
            Self::Spike { .. } => "spike",
            Self::Alert { .. } => "alert",
            Self::Frame { .. } => "frame",
            Self::GpuSample { .. } => "gpu_sample",
            Self::IrqEvent { .. } => "irq_event",
            Self::IoEvent { .. } => "io_event",
            Self::MigrationEvent { .. } => "migration_event",
            Self::CpuFreqSample { .. } => "cpu_freq_sample",
            Self::ForegroundEvent { .. } => "foreground_event",
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
            Self::SchedulerSample { .. } => None,
            Self::Spike { event } => event.elapsed_ms,
            Self::Alert { payload } => Some(payload.elapsed_ms),
            Self::Frame { event } => Some(event.elapsed_ms),
            Self::GpuSample { sample } => Some(sample.elapsed_ms),
            Self::IrqEvent { event } => event.elapsed_ms,
            Self::IoEvent { event } => Some(event.elapsed_ms),
            Self::MigrationEvent { event } => Some(event.elapsed_ms),
            Self::CpuFreqSample { event } => Some(event.elapsed_ms),
            Self::ForegroundEvent { event } => Some(event.elapsed_ms),
            Self::LiveDiagnosis { entry } => Some(entry.elapsed_ms),
            Self::DataQualityWarning { .. } => None,
            Self::FocusChanged { elapsed_ms, .. } => Some(*elapsed_ms),
            Self::FocusCleared { elapsed_ms, .. } => Some(*elapsed_ms),
            Self::Finished { .. } => None,
        }
    }
}

impl From<ForegroundEvent> for MonitorEvent {
    fn from(event: ForegroundEvent) -> Self {
        Self::ForegroundEvent {
            event: Box::new(event),
        }
    }
}

impl From<FocusEvent> for MonitorEvent {
    fn from(event: FocusEvent) -> Self {
        match event.action.as_str() {
            "cleared" => Self::FocusCleared {
                elapsed_ms: event.elapsed_ms,
                old_kind: None,
                reason: event
                    .reasons
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "focus cleared".to_owned()),
            },
            _ => Self::DataQualityWarning {
                message: "raw FocusEvent cannot be losslessly converted into MonitorEvent::FocusChanged because FocusGroupKind is not serialized in FocusEvent".to_owned(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use stutter_common::SchedulerEvent;

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

    #[test]
    fn scheduler_sample_event_reports_kind() {
        let event = MonitorEvent::SchedulerSample {
            event: Box::new(SchedulerEvent {
                kind: stutter_common::EVENT_RUNNABLE_LATENCY,
                tid: 123,
                cpu: 1,
                wakeup_target_cpu: 1,
                prio: 120,
                waker_tid: 0,
                target_pending_wakeups: 0,
                observed_runnable_depth: 0,
                maj_flt: 0,
                min_flt: 0,
                wakeup_ns: 10,
                switch_ns: 20,
                latency_ns: 10,
                comm: [0; 16],
                switch_prev_pid: 0,
                switch_prev_state: 0,
            }),
            comm: "test".to_owned(),
            label: "sample",
        };

        assert_eq!(event.kind(), "scheduler_sample");
        assert_eq!(event.elapsed_ms(), None);
    }
}
