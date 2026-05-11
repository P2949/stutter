use std::fmt;

use tokio::sync::mpsc;

use crate::{
    alert::AlertPayload,
    artifacts::ArtifactKind,
    events::push_artifact_event,
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

pub trait MonitorEventSink {
    fn on_event(&mut self, event: &MonitorEvent) -> Result<(), SinkError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MonitorOutputSinkKind {
    Recorder,
    Prometheus,
    Otel,
    Stdout,
    Alert,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MonitorOutputSinkRegistry {
    sinks: Vec<MonitorOutputSinkKind>,
}

impl MonitorOutputSinkRegistry {
    fn for_runtime(
        output: MonitorOutputConfig,
        recorder: &LiveRecorder,
        alert_sender: Option<&mpsc::Sender<AlertPayload>>,
    ) -> Self {
        let mut sinks = vec![MonitorOutputSinkKind::Recorder];

        if recorder.exporters.prometheus_state.is_some() {
            sinks.push(MonitorOutputSinkKind::Prometheus);
        }

        if recorder.exporters.otel_spike_tx.is_some() {
            sinks.push(MonitorOutputSinkKind::Otel);
        }

        if !output.json_stream || output.verbose || recorder.stdout_spike_stream.is_some() {
            sinks.push(MonitorOutputSinkKind::Stdout);
        }

        if alert_sender.is_some() {
            sinks.push(MonitorOutputSinkKind::Alert);
        }

        Self { sinks }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MonitorOutputConfig {
    pub json_stream: bool,
    pub verbose: bool,
}

pub struct MonitorOutputSinks<'a> {
    output: MonitorOutputConfig,
    recorder: &'a mut LiveRecorder,
    alert_sender: Option<&'a mpsc::Sender<AlertPayload>>,
    registry: MonitorOutputSinkRegistry,
}

impl<'a> MonitorOutputSinks<'a> {
    pub fn new(
        output: MonitorOutputConfig,
        recorder: &'a mut LiveRecorder,
        alert_sender: Option<&'a mpsc::Sender<AlertPayload>>,
    ) -> Self {
        let registry = MonitorOutputSinkRegistry::for_runtime(output, recorder, alert_sender);
        Self {
            output,
            recorder,
            alert_sender,
            registry,
        }
    }

    pub fn dispatch(&mut self, event: &MonitorEvent) -> Result<(), SinkError> {
        let sinks = self.registry.sinks.clone();
        for sink in sinks {
            self.dispatch_sink(sink, event)?;
        }
        Ok(())
    }

    fn dispatch_sink(
        &mut self,
        sink: MonitorOutputSinkKind,
        event: &MonitorEvent,
    ) -> Result<(), SinkError> {
        match sink {
            MonitorOutputSinkKind::Recorder => RecorderSink::new(self.recorder).on_event(event),
            MonitorOutputSinkKind::Prometheus => PrometheusSink::new(self.recorder).on_event(event),
            MonitorOutputSinkKind::Otel => OtelSink::new(self.recorder).on_event(event),
            MonitorOutputSinkKind::Stdout => {
                StdoutSink::new(self.output, self.recorder).on_event(event)
            }
            MonitorOutputSinkKind::Alert => {
                AlertSink::new(self.recorder, self.alert_sender).on_event(event)
            }
        }
    }

    pub fn dispatch_all<'b>(
        &mut self,
        events: impl IntoIterator<Item = &'b MonitorEvent>,
    ) -> Result<(), SinkError> {
        for event in events {
            self.dispatch(event)?;
        }
        Ok(())
    }
}

pub struct RecorderSink<'a> {
    recorder: &'a mut LiveRecorder,
}

impl<'a> RecorderSink<'a> {
    pub fn new(recorder: &'a mut LiveRecorder) -> Self {
        Self { recorder }
    }
}

impl MonitorEventSink for RecorderSink<'_> {
    fn on_event(&mut self, event: &MonitorEvent) -> Result<(), SinkError> {
        match event {
            MonitorEvent::Spike { event } => {
                if self.recorder.streams.contains(ArtifactKind::SpikeEvents) {
                    push_artifact_event(
                        self.recorder,
                        ArtifactKind::SpikeEvents,
                        event.as_ref(),
                        "spike_events",
                        |c| c.spike_event_count += 1,
                    );
                } else {
                    self.recorder.push_spike_event_to_buffer((**event).clone());
                }
            }
            MonitorEvent::IrqEvent { event } => {
                push_artifact_event(
                    self.recorder,
                    ArtifactKind::IrqEvents,
                    event.as_ref(),
                    "irq_events",
                    |c| c.irq_event_count += 1,
                );
            }
            MonitorEvent::IoEvent { event } => {
                push_artifact_event(
                    self.recorder,
                    ArtifactKind::BlockIoEvents,
                    event.as_ref(),
                    "io_events",
                    |c| c.block_io_event_count += 1,
                );
            }
            MonitorEvent::MigrationEvent { event } => {
                push_artifact_event(
                    self.recorder,
                    ArtifactKind::MigrationEvents,
                    event.as_ref(),
                    "migration_events",
                    |c| c.migration_event_count += 1,
                );
            }
            MonitorEvent::CpuFreqSample { event } => {
                push_artifact_event(
                    self.recorder,
                    ArtifactKind::CpuFreqSamples,
                    event.as_ref(),
                    "cpu_freq_samples",
                    |c| c.cpu_freq_sample_count += 1,
                );
            }
            MonitorEvent::GpuSample { sample } => {
                push_artifact_event(
                    self.recorder,
                    ArtifactKind::GpuSamples,
                    sample.as_ref(),
                    "gpu_samples",
                    |c| c.gpu_sample_count += 1,
                );
            }
            MonitorEvent::Frame { event } => {
                push_artifact_event(
                    self.recorder,
                    ArtifactKind::FrameEvents,
                    event.as_ref(),
                    "frame_events",
                    |c| c.frame_event_count += 1,
                );
            }
            MonitorEvent::ForegroundEvent { event } => {
                self.recorder.last_foreground_event = Some((**event).clone());
                push_artifact_event(
                    self.recorder,
                    ArtifactKind::ForegroundEvents,
                    event.as_ref(),
                    "foreground_events",
                    |c| c.foreground_event_count += 1,
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
                    root_pids: root_pids.to_vec(),
                    member_pids: member_pids.to_vec(),
                    confidence: *confidence,
                    score: *score,
                    situation: Some(*situation),
                    reasons: reasons.to_vec(),
                };
                push_artifact_event(
                    self.recorder,
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
                    self.recorder,
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

pub struct PrometheusSink<'a> {
    recorder: &'a mut LiveRecorder,
}

impl<'a> PrometheusSink<'a> {
    pub fn new(recorder: &'a mut LiveRecorder) -> Self {
        Self { recorder }
    }
}

impl MonitorEventSink for PrometheusSink<'_> {
    fn on_event(&mut self, event: &MonitorEvent) -> Result<(), SinkError> {
        let Some(state) = self.recorder.exporters.prometheus_state.as_ref() else {
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

pub struct OtelSink<'a> {
    recorder: &'a mut LiveRecorder,
}

impl<'a> OtelSink<'a> {
    pub fn new(recorder: &'a mut LiveRecorder) -> Self {
        Self { recorder }
    }
}

impl MonitorEventSink for OtelSink<'_> {
    fn on_event(&mut self, event: &MonitorEvent) -> Result<(), SinkError> {
        let MonitorEvent::Spike { event } = event else {
            return Ok(());
        };

        let Some(tx) = self.recorder.exporters.otel_spike_tx.as_ref() else {
            return Ok(());
        };

        let item = crate::otel::OtelSpike::from(event.as_ref());
        if tx.try_send(item).is_err()
            && let Some(dropped) = self.recorder.exporters.otel_spans_dropped.as_ref()
        {
            dropped.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }

        Ok(())
    }
}

pub struct StdoutSink<'a> {
    output: MonitorOutputConfig,
    recorder: &'a mut LiveRecorder,
}

impl<'a> StdoutSink<'a> {
    pub fn new(output: MonitorOutputConfig, recorder: &'a mut LiveRecorder) -> Self {
        Self { output, recorder }
    }
}

impl MonitorEventSink for StdoutSink<'_> {
    fn on_event(&mut self, event: &MonitorEvent) -> Result<(), SinkError> {
        match event {
            MonitorEvent::SchedulerSample { event, comm, label }
                if !self.output.json_stream
                    && (*label == "spike" || (self.output.verbose && *label == "sample")) =>
            {
                print_event(event.as_ref(), comm, label);
            }
            MonitorEvent::Spike { event } => {
                if let Some(stream) = self.recorder.stdout_spike_stream.as_mut()
                    && let Err(err) = stream.push(event.as_ref())
                {
                    log::warn!("json_stream_write_failed err={err:#}");
                    self.recorder.counters.stdout_spike_stream_errors += 1;
                }
            }
            _ => {}
        }

        Ok(())
    }
}

pub struct AlertSink<'a> {
    recorder: &'a mut LiveRecorder,
    alert_sender: Option<&'a mpsc::Sender<AlertPayload>>,
}

impl<'a> AlertSink<'a> {
    pub fn new(
        recorder: &'a mut LiveRecorder,
        alert_sender: Option<&'a mpsc::Sender<AlertPayload>>,
    ) -> Self {
        Self {
            recorder,
            alert_sender,
        }
    }
}

impl MonitorEventSink for AlertSink<'_> {
    fn on_event(&mut self, event: &MonitorEvent) -> Result<(), SinkError> {
        let MonitorEvent::Alert { payload } = event else {
            return Ok(());
        };

        let Some(sender) = self.alert_sender else {
            return Ok(());
        };

        if let Err(err) = sender.try_send((**payload).clone()) {
            match err {
                mpsc::error::TrySendError::Full(_) => {
                    log::warn!("alert_channel_full_dropping_alert");
                    self.recorder.counters.alert_events_dropped_count += 1;
                }
                mpsc::error::TrySendError::Closed(_) => {
                    log::warn!("alert_channel_closed");
                    self.recorder.counters.alert_channel_closed_count += 1;
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

        assert_eq!(
            registry.sinks,
            vec![
                MonitorOutputSinkKind::Recorder,
                MonitorOutputSinkKind::Stdout,
            ]
        );
    }

    #[test]
    fn output_sink_registry_includes_exporters_and_alert_when_active() {
        let output = MonitorOutputConfig {
            json_stream: true,
            verbose: false,
        };
        let mut recorder = LiveRecorder::default();
        recorder.exporters.prometheus_state = Some(std::sync::Arc::new(
            crate::prometheus::PrometheusState::new_started_now(),
        ));
        recorder.stdout_spike_stream = Some(crate::recorder::StdoutJsonStream::new());

        let (alert_tx, _alert_rx) = mpsc::channel(1);
        let registry = MonitorOutputSinkRegistry::for_runtime(output, &recorder, Some(&alert_tx));

        assert!(registry.sinks.contains(&MonitorOutputSinkKind::Recorder));
        assert!(registry.sinks.contains(&MonitorOutputSinkKind::Prometheus));
        assert!(registry.sinks.contains(&MonitorOutputSinkKind::Stdout));
        assert!(registry.sinks.contains(&MonitorOutputSinkKind::Alert));
        assert!(!registry.sinks.contains(&MonitorOutputSinkKind::Otel));
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

        RecorderSink::new(&mut recorder).on_event(&event).unwrap();

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

        AlertSink::new(&mut recorder, Some(&tx))
            .on_event(&event)
            .unwrap();

        assert_eq!(recorder.counters.alert_events_dropped_count, 1);
    }
}
