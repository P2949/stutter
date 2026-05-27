use super::{
    error::SinkError,
    model::{MonitorEventSink, MonitorSinkContext},
};
use crate::session_events::MonitorEvent;

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
