use std::fmt;

use tokio::sync::mpsc;

use crate::{
    alert::AlertPayload,
    artifacts::{ArtifactKind, push_artifact_event},
    metrics::print_event,
    recorder::{self, LiveRecorder},
    session_events::MonitorEvent,
};

#[derive(Debug)]
pub struct SinkError {
    pub sink: &'static str,
    pub event_kind: &'static str,
    pub message: String,
}

impl SinkError {
    pub fn new(
        sink: &'static str,
        event_kind: &'static str,
        error: impl std::fmt::Display,
    ) -> Self {
        Self {
            sink,
            event_kind,
            message: error.to_string(),
        }
    }
}

impl fmt::Display for SinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "monitor event sink {} failed for {}: {}",
            self.sink, self.event_kind, self.message
        )
    }
}

impl std::error::Error for SinkError {}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MonitorOutputConfig {
    pub json_stream: bool,
    pub verbose: bool,
    pub retain_interval_limit: Option<usize>,
    pub count_interval_retention_drops: bool,
}

pub struct MonitorSinkContext<'a> {
    pub recorder: &'a mut LiveRecorder,
    pub alert_sender: Option<&'a mpsc::Sender<AlertPayload>>,
    pub output: MonitorOutputConfig,
}

pub trait MonitorEventSink: Send {
    fn name(&self) -> &'static str;

    fn on_event(
        &mut self,
        event: &MonitorEvent,
        ctx: &mut MonitorSinkContext<'_>,
    ) -> Result<(), SinkError>;
}

pub struct MonitorOutputSinkRegistry {
    sinks: Vec<Box<dyn MonitorEventSink + Send>>,
}

impl MonitorOutputSinkRegistry {
    pub fn for_runtime(
        output: MonitorOutputConfig,
        recorder: &LiveRecorder,
        alert_sender: Option<&mpsc::Sender<AlertPayload>>,
    ) -> Self {
        let mut sinks: Vec<Box<dyn MonitorEventSink + Send>> = vec![Box::new(RecorderSink::new())];

        if recorder.exporters.prometheus_state.is_some() {
            sinks.push(Box::new(PrometheusSink::new()));
        }

        if recorder.exporters.otel_spike_tx.is_some() {
            sinks.push(Box::new(OtelSink::new()));
        }

        if !output.json_stream || output.verbose || recorder.stdout_spike_stream.is_some() {
            sinks.push(Box::new(StdoutSink::new()));
        }

        if alert_sender.is_some() {
            sinks.push(Box::new(AlertSink::new()));
        }

        Self { sinks }
    }

    pub fn dispatch(
        &mut self,
        event: &MonitorEvent,
        ctx: &mut MonitorSinkContext<'_>,
    ) -> Result<(), SinkError> {
        for sink in &mut self.sinks {
            sink.on_event(event, ctx)?;
        }
        Ok(())
    }

    pub fn dispatch_all<'a>(
        &mut self,
        events: impl IntoIterator<Item = &'a MonitorEvent>,
        ctx: &mut MonitorSinkContext<'_>,
    ) -> Result<(), SinkError> {
        for event in events {
            self.dispatch(event, ctx)?;
        }
        Ok(())
    }

    pub fn sink_names(&self) -> Vec<&'static str> {
        self.sinks.iter().map(|sink| sink.name()).collect()
    }
}

pub struct MonitorOutputSinks<'a, 'b> {
    ctx: MonitorSinkContext<'a>,
    registry: &'b mut MonitorOutputSinkRegistry,
}

impl<'a, 'b> MonitorOutputSinks<'a, 'b> {
    pub fn new(
        output: MonitorOutputConfig,
        recorder: &'a mut LiveRecorder,
        alert_sender: Option<&'a mpsc::Sender<AlertPayload>>,
        registry: &'b mut MonitorOutputSinkRegistry,
    ) -> Self {
        Self {
            ctx: MonitorSinkContext {
                recorder,
                alert_sender,
                output,
            },
            registry,
        }
    }

    pub fn dispatch(&mut self, event: &MonitorEvent) -> Result<(), SinkError> {
        self.dispatch_all(std::iter::once(event))
    }

    pub fn dispatch_all<'c>(
        &mut self,
        events: impl IntoIterator<Item = &'c MonitorEvent>,
    ) -> Result<(), SinkError> {
        self.registry.dispatch_all(events, &mut self.ctx)
    }
}

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
                    root_pids: root_pids.to_vec(),
                    member_pids: member_pids.to_vec(),
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

pub struct PrometheusSink;

impl PrometheusSink {
    pub fn new() -> Self {
        Self
    }
}

impl Default for PrometheusSink {
    fn default() -> Self {
        Self::new()
    }
}

impl MonitorEventSink for PrometheusSink {
    fn name(&self) -> &'static str {
        "prometheus"
    }

    fn on_event(
        &mut self,
        event: &MonitorEvent,
        ctx: &mut MonitorSinkContext<'_>,
    ) -> Result<(), SinkError> {
        let Some(state) = ctx.recorder.exporters.prometheus_state.as_ref() else {
            return Ok(());
        };

        match event {
            MonitorEvent::SchedulerSample { event, .. } => {
                state.inc_samples(1);
                state.observe_latency_ns(event.latency_ns);
            }
            MonitorEvent::Spike { .. } => {
                state.inc_spikes(1);
            }
            _ => {}
        }

        Ok(())
    }
}

pub struct OtelSink;

impl OtelSink {
    pub fn new() -> Self {
        Self
    }
}

impl Default for OtelSink {
    fn default() -> Self {
        Self::new()
    }
}

impl MonitorEventSink for OtelSink {
    fn name(&self) -> &'static str {
        "otel"
    }

    fn on_event(
        &mut self,
        event: &MonitorEvent,
        ctx: &mut MonitorSinkContext<'_>,
    ) -> Result<(), SinkError> {
        let MonitorEvent::Spike { event } = event else {
            return Ok(());
        };

        let Some(tx) = ctx.recorder.exporters.otel_spike_tx.as_ref() else {
            return Ok(());
        };

        let item = crate::otel::OtelSpike::from(event.as_ref());
        if tx.try_send(item).is_err()
            && let Some(dropped) = ctx.recorder.exporters.otel_spans_dropped.as_ref()
        {
            dropped.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }

        Ok(())
    }
}

pub struct StdoutSink;

impl StdoutSink {
    pub fn new() -> Self {
        Self
    }
}

impl Default for StdoutSink {
    fn default() -> Self {
        Self::new()
    }
}

impl MonitorEventSink for StdoutSink {
    fn name(&self) -> &'static str {
        "stdout"
    }

    fn on_event(
        &mut self,
        event: &MonitorEvent,
        ctx: &mut MonitorSinkContext<'_>,
    ) -> Result<(), SinkError> {
        match event {
            MonitorEvent::SchedulerSample { event, comm, label }
                if !ctx.output.json_stream
                    && (*label == "spike" || (ctx.output.verbose && *label == "sample")) =>
            {
                print_event(event.as_ref(), comm, label);
            }
            MonitorEvent::Spike { event } => {
                if let Some(stream) = ctx.recorder.stdout_spike_stream.as_mut()
                    && let Err(err) = stream.push(event.as_ref())
                {
                    log::warn!("json_stream_write_failed err={err:#}");
                    ctx.recorder.counters.stdout_spike_stream_errors += 1;
                }
            }
            _ => {}
        }

        Ok(())
    }
}

pub struct AlertSink;

impl AlertSink {
    pub fn new() -> Self {
        Self
    }
}

impl Default for AlertSink {
    fn default() -> Self {
        Self::new()
    }
}

impl MonitorEventSink for AlertSink {
    fn name(&self) -> &'static str {
        "alert"
    }

    fn on_event(
        &mut self,
        event: &MonitorEvent,
        ctx: &mut MonitorSinkContext<'_>,
    ) -> Result<(), SinkError> {
        let MonitorEvent::Alert { payload } = event else {
            return Ok(());
        };

        let Some(sender) = ctx.alert_sender else {
            return Ok(());
        };

        if let Err(err) = sender.try_send((**payload).clone()) {
            match err {
                mpsc::error::TrySendError::Full(_) => {
                    log::warn!("alert_channel_full_dropping_alert");
                    ctx.recorder.counters.alert_events_dropped_count += 1;
                }
                mpsc::error::TrySendError::Closed(_) => {
                    log::warn!("alert_channel_closed");
                    ctx.recorder.counters.alert_channel_closed_count += 1;
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recorder::SpikeEvent;

    #[test]
    fn output_sink_registry_includes_only_active_optional_sinks() {
        let output = MonitorOutputConfig::default();
        let recorder = LiveRecorder::default();
        let registry = MonitorOutputSinkRegistry::for_runtime(output, &recorder, None);

        assert_eq!(registry.sink_names(), vec!["recorder", "stdout"]);
    }

    #[test]
    fn output_sink_registry_includes_exporters_and_alert_when_active() {
        let output = MonitorOutputConfig {
            json_stream: true,
            verbose: false,
            ..MonitorOutputConfig::default()
        };
        let mut recorder = LiveRecorder::default();
        recorder.exporters.prometheus_state = Some(std::sync::Arc::new(
            crate::prometheus::PrometheusState::new_started_now(),
        ));
        recorder.stdout_spike_stream = Some(crate::recorder::StdoutJsonStream::new());

        let (alert_tx, _alert_rx) = mpsc::channel(1);
        let registry = MonitorOutputSinkRegistry::for_runtime(output, &recorder, Some(&alert_tx));
        let names = registry.sink_names();

        assert!(names.contains(&"recorder"));
        assert!(names.contains(&"prometheus"));
        assert!(names.contains(&"stdout"));
        assert!(names.contains(&"alert"));
        assert!(!names.contains(&"otel"));
    }

    #[test]
    fn recorder_sink_stores_spike_in_buffer_when_stream_is_absent() {
        let mut recorder = LiveRecorder::default();
        recorder.buffers.spike_events =
            Some(crate::recorder::SpikeEventBuffer::with_max_events(10));

        let spike = SpikeEvent {
            elapsed_ms: Some(5),
            task: 10,
            latency_ns: 2_000_000,
            ..Default::default()
        };

        let event = MonitorEvent::Spike {
            event: Box::new(spike),
        };

        let mut ctx = MonitorSinkContext {
            recorder: &mut recorder,
            alert_sender: None,
            output: MonitorOutputConfig::default(),
        };
        let mut sink = RecorderSink::new();
        sink.on_event(&event, &mut ctx).unwrap();

        assert_eq!(
            recorder
                .buffers
                .spike_events
                .as_ref()
                .unwrap()
                .as_slice()
                .len(),
            1
        );
    }

    #[test]
    fn alert_sink_counts_full_channel_drops() {
        let mut recorder = LiveRecorder::default();
        let (tx, _rx) = mpsc::channel(1);
        // fill the channel
        let payload = AlertPayload {
            title: "title".to_owned(),
            message: "message".to_owned(),
            task: 1,
            active: true,
            class: crate::process_tree::TaskClass::Unknown,
            comm: "task".to_owned(),
            process_pid: None,
            process_comm: String::new(),
            latency_ns: 1_000_000,
            latency_ms: 1,
            cpu: 0,
            prio: 120,
            wakeup_ns: 1,
            switch_ns: 2,
            elapsed_ms: 3,
            scx_ops: None,
            scx_state: None,
            scx_enable_seq: None,
        };
        tx.try_send(payload.clone()).unwrap();

        let event = MonitorEvent::Alert {
            payload: Box::new(payload),
        };

        let mut ctx = MonitorSinkContext {
            recorder: &mut recorder,
            alert_sender: Some(&tx),
            output: MonitorOutputConfig::default(),
        };
        let mut sink = AlertSink::new();
        sink.on_event(&event, &mut ctx).unwrap();

        assert_eq!(recorder.counters.alert_events_dropped_count, 1);
    }
}
