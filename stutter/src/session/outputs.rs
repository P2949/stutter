pub struct OutputRuntime {
    pub recorder: crate::recorder::LiveRecorder,
    pub prometheus_state: Option<std::sync::Arc<crate::prometheus::PrometheusState>>,
    pub prometheus_task: Option<tokio::task::JoinHandle<()>>,
    pub otel_exporter: Option<crate::otel::OtelExporterHandle>,
    pub alert_sender: Option<tokio::sync::mpsc::Sender<crate::alert::AlertPayload>>,
    pub sink_registry: crate::session::sinks::MonitorOutputSinkRegistry,
}

impl OutputRuntime {
    pub fn new(recorder: crate::recorder::LiveRecorder) -> Self {
        let sink_registry = crate::session::sinks::MonitorOutputSinkRegistry::for_runtime(
            crate::session::sinks::MonitorOutputConfig::default(),
            &recorder,
            None,
        );
        Self {
            recorder,
            prometheus_state: None,
            prometheus_task: None,
            otel_exporter: None,
            alert_sender: None,
            sink_registry,
        }
    }

    pub fn from_parts(
        recorder: crate::recorder::LiveRecorder,
        prometheus_state: Option<std::sync::Arc<crate::prometheus::PrometheusState>>,
        prometheus_task: Option<tokio::task::JoinHandle<()>>,
        otel_exporter: Option<crate::otel::OtelExporterHandle>,
        alert_sender: Option<tokio::sync::mpsc::Sender<crate::alert::AlertPayload>>,
        output: crate::session::sinks::MonitorOutputConfig,
    ) -> Self {
        let sink_registry = crate::session::sinks::MonitorOutputSinkRegistry::for_runtime(
            output,
            &recorder,
            alert_sender.as_ref(),
        );
        Self {
            recorder,
            prometheus_state,
            prometheus_task,
            otel_exporter,
            alert_sender,
            sink_registry,
        }
    }
}
