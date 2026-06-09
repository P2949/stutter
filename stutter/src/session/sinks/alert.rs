use tokio::sync::mpsc;

use super::{
    error::SinkError,
    model::{MonitorEventSink, MonitorSinkContext},
};
use crate::session_events::MonitorEvent;

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
