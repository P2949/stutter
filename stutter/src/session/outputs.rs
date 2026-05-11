pub struct OutputRuntime {
    pub recorder: crate::recorder::LiveRecorder,
    pub prometheus_state: Option<std::sync::Arc<crate::prometheus::PrometheusState>>,
    pub prometheus_task: Option<tokio::task::JoinHandle<()>>,
    pub otel_exporter: Option<crate::otel::OtelExporterHandle>,
    pub alert_sender: Option<tokio::sync::mpsc::Sender<crate::alert::AlertPayload>>,
}

impl OutputRuntime {
    pub fn new(recorder: crate::recorder::LiveRecorder) -> Self {
        Self {
            recorder,
            prometheus_state: None,
            prometheus_task: None,
            otel_exporter: None,
            alert_sender: None,
        }
    }

    pub fn from_parts(
        recorder: crate::recorder::LiveRecorder,
        prometheus_state: Option<std::sync::Arc<crate::prometheus::PrometheusState>>,
        prometheus_task: Option<tokio::task::JoinHandle<()>>,
        otel_exporter: Option<crate::otel::OtelExporterHandle>,
        alert_sender: Option<tokio::sync::mpsc::Sender<crate::alert::AlertPayload>>,
    ) -> Self {
        Self {
            recorder,
            prometheus_state,
            prometheus_task,
            otel_exporter,
            alert_sender,
        }
    }
}
