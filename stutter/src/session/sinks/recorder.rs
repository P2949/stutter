use stutter_core::ids::Pid;

use super::{
    error::SinkError,
    model::{MonitorEventSink, MonitorSinkContext},
};
use crate::{
    artifacts::{ArtifactKind, push_artifact_event},
    recorder,
    session_events::MonitorEvent,
};

pub struct RecorderSink;

impl RecorderSink {
    pub fn new() -> Self {
        Self
    }
}

impl Default for RecorderSink {
    fn default() -> Self {
        Self::new()
    }
}

impl MonitorEventSink for RecorderSink {
    fn name(&self) -> &'static str {
        "recorder"
    }

    fn on_event(
        &mut self,
        event: &MonitorEvent,
        ctx: &mut MonitorSinkContext<'_>,
    ) -> Result<(), SinkError> {
        match event {
            MonitorEvent::Interval { records, .. } => {
                ctx.recorder.counters.interval_record_count += records.len() as u64;

                if ctx.recorder.streams.contains(ArtifactKind::Interval) {
                    for record in records {
                        ctx.recorder
                            .streams
                            .push(ArtifactKind::Interval, record)
                            .map_err(|err| SinkError::new(self.name(), event.kind(), err))?;
                    }
                } else if let Some(max_intervals) = ctx.output.retain_interval_limit {
                    ctx.recorder
                        .buffers
                        .interval_records
                        .extend(records.iter().cloned());

                    if ctx.recorder.buffers.interval_records.len() > max_intervals {
                        let drop_count =
                            ctx.recorder.buffers.interval_records.len() - max_intervals;
                        ctx.recorder.buffers.interval_records.drain(0..drop_count);
                        if ctx.output.count_interval_retention_drops {
                            ctx.recorder.counters.intervals_dropped += drop_count as u64;
                        }
                    }
                }

                if let Some(writer) = ctx.recorder.csv_writer.as_mut() {
                    for record in records {
                        writer
                            .push(record)
                            .map_err(|err| SinkError::new(self.name(), event.kind(), err))?;
                    }
                }
            }
            MonitorEvent::Spike { event } => {
                if ctx.recorder.streams.contains(ArtifactKind::SpikeEvents) {
                    push_artifact_event(
                        ctx.recorder,
                        ArtifactKind::SpikeEvents,
                        event.as_ref(),
                        "spike_events",
                        |c| c.spike_event_count += 1,
                    );
                } else {
                    ctx.recorder.push_spike_event_to_buffer((**event).clone());
                }
            }
            MonitorEvent::IrqEvent { event } => {
                push_artifact_event(
                    ctx.recorder,
                    ArtifactKind::IrqEvents,
                    event.as_ref(),
                    "irq_events",
                    |c| c.irq_event_count += 1,
                );
            }
            MonitorEvent::IoEvent { event } => {
                push_artifact_event(
                    ctx.recorder,
                    ArtifactKind::BlockIoEvents,
                    event.as_ref(),
                    "io_events",
                    |c| c.block_io_event_count += 1,
                );
            }
            MonitorEvent::MigrationEvent { event } => {
                push_artifact_event(
                    ctx.recorder,
                    ArtifactKind::MigrationEvents,
                    event.as_ref(),
                    "migration_events",
                    |c| c.migration_event_count += 1,
                );
            }
            MonitorEvent::CpuFreqSample { event } => {
                push_artifact_event(
                    ctx.recorder,
                    ArtifactKind::CpuFreqSamples,
                    event.as_ref(),
                    "cpu_freq_samples",
                    |c| c.cpu_freq_sample_count += 1,
                );
            }
            MonitorEvent::GpuSample { sample } => {
                push_artifact_event(
                    ctx.recorder,
                    ArtifactKind::GpuSamples,
                    sample.as_ref(),
                    "gpu_samples",
                    |c| c.gpu_sample_count += 1,
                );
            }
            MonitorEvent::GpuEngineSample { sample } => {
                push_artifact_event(
                    ctx.recorder,
                    ArtifactKind::GpuEngineSamples,
                    sample.as_ref(),
                    "gpu_engine_samples",
                    |c| c.gpu_engine_sample_count += 1,
                );
            }
            MonitorEvent::Frame { event } => {
                push_artifact_event(
                    ctx.recorder,
                    ArtifactKind::FrameEvents,
                    event.as_ref(),
                    "frame_events",
                    |c| c.frame_event_count += 1,
                );
            }
            MonitorEvent::ForegroundEvent { event } => {
                ctx.recorder.last_foreground_event = Some((**event).clone());
                push_artifact_event(
                    ctx.recorder,
                    ArtifactKind::ForegroundEvents,
                    event.as_ref(),
                    "foreground_events",
                    |c| c.foreground_event_count += 1,
                );
            }
            MonitorEvent::ScxEvent { event } => {
                if ctx.recorder.streams.contains(ArtifactKind::ScxEvents) {
                    push_artifact_event(
                        ctx.recorder,
                        ArtifactKind::ScxEvents,
                        event.as_ref(),
                        "scx_events",
                        |c| c.scx_event_count += 1,
                    );
                } else {
                    ctx.recorder.buffers.scx_events.push((**event).clone());
                    ctx.recorder.counters.scx_event_count += 1;
                }
            }
            MonitorEvent::KmsFlipEvent { event } => {
                push_artifact_event(
                    ctx.recorder,
                    ArtifactKind::KmsFlipEvents,
                    event.as_ref(),
                    "kms_flip_events",
                    |c| c.kms_flip_event_count += 1,
                );
            }
            MonitorEvent::DrmFenceEvent { event } => {
                push_artifact_event(
                    ctx.recorder,
                    ArtifactKind::DrmFenceEvents,
                    event.as_ref(),
                    "drm_fence_events",
                    |c| c.drm_fence_event_count += 1,
                );
            }
            MonitorEvent::WaylandPresentationEvent { event } => {
                push_artifact_event(
                    ctx.recorder,
                    ArtifactKind::WaylandPresentationEvents,
                    event.as_ref(),
                    "wayland_presentation_events",
                    |c| c.wayland_presentation_event_count += 1,
                );
            }
            MonitorEvent::DmaBufEvent { event } => {
                push_artifact_event(
                    ctx.recorder,
                    ArtifactKind::DmaBufEvents,
                    event.as_ref(),
                    "dmabuf_events",
                    |c| c.dmabuf_event_count += 1,
                );
            }
            MonitorEvent::FocusChanged {
                elapsed_ms,
                old_kind,
                new_kind,
                root_pids,
                member_pids,
                confidence,
                score,
                situation,
                reasons,
            } => {
                let event = recorder::FocusEvent {
                    elapsed_ms: *elapsed_ms,
                    action: "changed".to_owned(),
                    old_kind: old_kind.map(|kind| format!("{kind:?}")),
                    kind: Some(format!("{new_kind:?}")),
                    root_pids: root_pids.iter().copied().map(Pid::new).collect(),
                    member_pids: member_pids.iter().copied().map(Pid::new).collect(),
                    confidence: *confidence,
                    score: *score,
                    situation: Some(*situation),
                    reasons: reasons.to_vec(),
                };
                push_artifact_event(
                    ctx.recorder,
                    ArtifactKind::FocusEvents,
                    &event,
                    "focus_events",
                    |c| c.focus_event_count += 1,
                );
            }
            MonitorEvent::FocusCleared {
                elapsed_ms,
                old_kind,
                reason,
            } => {
                let event = recorder::FocusEvent {
                    elapsed_ms: *elapsed_ms,
                    action: "cleared".to_owned(),
                    old_kind: old_kind.map(|kind| format!("{kind:?}")),
                    kind: None,
                    root_pids: Vec::new(),
                    member_pids: Vec::new(),
                    confidence: 0.0,
                    score: 0.0,
                    situation: None,
                    reasons: vec![reason.clone()],
                };
                push_artifact_event(
                    ctx.recorder,
                    ArtifactKind::FocusEvents,
                    &event,
                    "focus_events",
                    |c| c.focus_event_count += 1,
                );
            }
            _ => {}
        }
        Ok(())
    }
}
