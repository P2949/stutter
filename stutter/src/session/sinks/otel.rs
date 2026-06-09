use super::{
    error::SinkError,
    model::{MonitorEventSink, MonitorSinkContext},
};
use crate::session_events::MonitorEvent;

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
