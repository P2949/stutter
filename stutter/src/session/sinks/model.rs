use tokio::sync::mpsc;

use super::error::SinkError;
use crate::{alert::AlertPayload, recorder::LiveRecorder, session_events::MonitorEvent};

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
