#![allow(dead_code)]

pub struct OutputRuntime {
    pub recorder: crate::recorder::LiveRecorder,
    pub prometheus_state: Option<std::sync::Arc<crate::prometheus::PrometheusState>>,
    pub prometheus_task: Option<tokio::task::JoinHandle<()>>,
    pub otel_exporter: Option<crate::otel::OtelExporterHandle>,
}

impl OutputRuntime {
    pub fn new(recorder: crate::recorder::LiveRecorder) -> Self {
        Self {
            recorder,
            prometheus_state: None,
            prometheus_task: None,
            otel_exporter: None,
        }
    }
}
