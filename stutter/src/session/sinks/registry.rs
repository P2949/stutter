use tokio::sync::mpsc;

use super::{
    alert::AlertSink,
    error::SinkError,
    model::{MonitorEventSink, MonitorOutputConfig, MonitorSinkContext},
    otel::OtelSink,
    prometheus::PrometheusSink,
    recorder::RecorderSink,
    stdout::StdoutSink,
};
use crate::{alert::AlertPayload, recorder::LiveRecorder, session_events::MonitorEvent};

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
