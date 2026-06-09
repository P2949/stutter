pub struct OutputRuntime {
    pub alert_sender: Option<tokio::sync::mpsc::Sender<crate::alert::AlertPayload>>,
    pub sink_registry: crate::session::sinks::MonitorOutputSinkRegistry,
}

impl OutputRuntime {
    pub fn new(sink_registry: crate::session::sinks::MonitorOutputSinkRegistry) -> Self {
        Self {
            alert_sender: None,
            sink_registry,
        }
    }

    pub fn from_parts(
        alert_sender: Option<tokio::sync::mpsc::Sender<crate::alert::AlertPayload>>,
        sink_registry: crate::session::sinks::MonitorOutputSinkRegistry,
    ) -> Self {
        Self {
            alert_sender,
            sink_registry,
        }
    }
}
