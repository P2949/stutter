use super::{
    error::SinkError,
    model::{MonitorEventSink, MonitorSinkContext},
};
use crate::{metrics::print_event, session_events::MonitorEvent};

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
