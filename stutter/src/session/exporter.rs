//! Metrics and OpenTelemetry exporter setup for monitor sessions.

use std::{sync::Arc, time::SystemTime};

use log::{info, warn};

use crate::{
    config::model::MonitorConfig,
    recorder::{self, LiveRecorder},
};

pub(crate) struct ExporterRuntime {
    pub(crate) prometheus_state: Option<Arc<crate::prometheus::PrometheusState>>,
    pub(crate) prometheus_task: Option<tokio::task::JoinHandle<()>>,
    pub(crate) otel_exporter: Option<crate::otel::OtelExporterHandle>,
}

impl ExporterRuntime {
    pub(crate) async fn begin(
        config: &MonitorConfig,
        recorder: &mut LiveRecorder,
    ) -> anyhow::Result<Self> {
        let (prometheus_state, prometheus_task) = if let Some(port) = config.outputs.metrics_port {
            let state = Arc::new(crate::prometheus::PrometheusState::new_started_now());
            let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
            let task = crate::prometheus::spawn_metrics_server(addr, state.clone()).await?;
            info!("prometheus metrics listening on http://127.0.0.1:{port}/metrics");
            (Some(state), Some(task))
        } else {
            (None, None)
        };

        recorder.exporters.prometheus_state = prometheus_state.clone();

        let mut otel_exporter = None;
        if let Some(endpoint) = config.outputs.otlp_endpoint.as_ref() {
            let started_at = recorder
                .run
                .as_ref()
                .map(|r| r.started_at)
                .unwrap_or_else(SystemTime::now);
            let monotonic_start_ns = recorder
                .run
                .as_ref()
                .and_then(|r| r.monotonic_start_ns)
                .unwrap_or_else(|| recorder::monotonic_now_ns().unwrap_or(0));

            let otel_config = crate::otel::OtelConfig {
                endpoint: endpoint.clone(),
                service_name: config.outputs.otel_service_name.clone(),
                started_at,
                monotonic_start_ns,
            };

            match crate::otel::spawn_exporter(otel_config) {
                Ok(handle) => {
                    recorder.exporters.otel_spike_tx = Some(handle.tx.clone());
                    recorder.exporters.otel_spans_dropped = Some(handle.dropped.clone());
                    otel_exporter = Some(handle);
                }
                Err(err) => {
                    warn!("failed to start OTel exporter: {err:#}");
                }
            }
        }

        Ok(Self {
            prometheus_state,
            prometheus_task,
            otel_exporter,
        })
    }
}
