use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[derive(Default, Debug)]
pub struct PrometheusState {
    pub start_unix_seconds: AtomicU64,
    pub total_spikes: AtomicU64,
    pub total_samples: AtomicU64,
    pub max_latency_ns: AtomicU64,
    pub latest_p99_ns: AtomicU64,
    pub event_stream_write_errors: AtomicU64,
    pub ebpf_ringbuf_drops: AtomicU64,
    pub active_targets: AtomicU64,
}

impl PrometheusState {
    pub fn new_started_now() -> Self {
        let start_unix_seconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0);

        Self {
            start_unix_seconds: AtomicU64::new(start_unix_seconds),
            ..Self::default()
        }
    }

    pub fn inc_samples(&self, count: u64) {
        self.total_samples.fetch_add(count, Ordering::Relaxed);
    }

    pub fn inc_spikes(&self, count: u64) {
        self.total_spikes.fetch_add(count, Ordering::Relaxed);
    }

    pub fn observe_latency_ns(&self, latency_ns: u64) {
        let mut current = self.max_latency_ns.load(Ordering::Relaxed);

        while latency_ns > current {
            match self.max_latency_ns.compare_exchange_weak(
                current,
                latency_ns,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(next) => current = next,
            }
        }
    }

    pub fn set_latest_p99_ns(&self, value: u64) {
        self.latest_p99_ns.store(value, Ordering::Relaxed);
    }

    pub fn set_active_targets(&self, value: u64) {
        self.active_targets.store(value, Ordering::Relaxed);
    }

    pub fn set_event_stream_write_errors(&self, value: u64) {
        self.event_stream_write_errors
            .store(value, Ordering::Relaxed);
    }

    pub fn set_ebpf_ringbuf_drops(&self, value: u64) {
        self.ebpf_ringbuf_drops.store(value, Ordering::Relaxed);
    }
}

pub fn render_metrics(state: &PrometheusState) -> String {
    let start_unix_seconds = state.start_unix_seconds.load(Ordering::Relaxed);
    let total_spikes = state.total_spikes.load(Ordering::Relaxed);
    let total_samples = state.total_samples.load(Ordering::Relaxed);
    let max_latency_ns = state.max_latency_ns.load(Ordering::Relaxed);
    let latest_p99_ns = state.latest_p99_ns.load(Ordering::Relaxed);
    let active_targets = state.active_targets.load(Ordering::Relaxed);
    let event_stream_write_errors = state.event_stream_write_errors.load(Ordering::Relaxed);
    let ebpf_ringbuf_drops = state.ebpf_ringbuf_drops.load(Ordering::Relaxed);

    let mut output = format!(
        concat!(
            "# HELP stutter_start_unix_seconds Unix timestamp when this stutter monitor session started.\n",
            "# TYPE stutter_start_unix_seconds gauge\n",
            "stutter_start_unix_seconds {start_unix_seconds}\n",
            "# HELP stutter_spikes_total Total scheduler spike events detected since monitor start.\n",
            "# TYPE stutter_spikes_total counter\n",
            "stutter_spikes_total {total_spikes}\n",
            "# HELP stutter_samples_total Total scheduler events processed since monitor start.\n",
            "# TYPE stutter_samples_total counter\n",
            "stutter_samples_total {total_samples}\n",
            "# HELP stutter_max_latency_ns Maximum scheduler latency observed so far, in nanoseconds.\n",
            "# TYPE stutter_max_latency_ns gauge\n",
            "stutter_max_latency_ns {max_latency_ns}\n",
            "# HELP stutter_latest_p99_ns Latest calculated p99 scheduler latency, in nanoseconds.\n",
            "# TYPE stutter_latest_p99_ns gauge\n",
            "stutter_latest_p99_ns {latest_p99_ns}\n",
            "# HELP stutter_active_targets Current number of active monitored targets.\n",
            "# TYPE stutter_active_targets gauge\n",
            "stutter_active_targets {active_targets}\n",
            "# HELP stutter_event_stream_write_errors_total Total event stream write errors since monitor start.\n",
            "# TYPE stutter_event_stream_write_errors_total counter\n",
            "stutter_event_stream_write_errors_total {event_stream_write_errors}\n",
            "# HELP stutter_ebpf_drops_total Total eBPF/ring-buffer drops observed since monitor start.\n",
            "# TYPE stutter_ebpf_drops_total counter\n",
            "stutter_ebpf_drops_total {ebpf_ringbuf_drops}\n",
        ),
        start_unix_seconds = start_unix_seconds,
        total_spikes = total_spikes,
        total_samples = total_samples,
        max_latency_ns = max_latency_ns,
        latest_p99_ns = latest_p99_ns,
        active_targets = active_targets,
        event_stream_write_errors = event_stream_write_errors,
        ebpf_ringbuf_drops = ebpf_ringbuf_drops,
    );

    output.push_str(
        &crate::autotune::prometheus_metrics::render_default_autotune_prometheus_metrics(),
    );
    output
}

pub async fn spawn_metrics_server(
    addr: std::net::SocketAddr,
    state: Arc<PrometheusState>,
) -> anyhow::Result<tokio::task::JoinHandle<()>> {
    let listener = TcpListener::bind(addr).await?;

    let handle = tokio::spawn(async move {
        serve_metrics_with_listener(listener, state).await;
    });

    Ok(handle)
}

async fn serve_metrics_with_listener(listener: TcpListener, state: Arc<PrometheusState>) {
    loop {
        let (mut socket, _) = match listener.accept().await {
            Ok(s) => s,
            Err(_) => continue,
        };

        let state = state.clone();

        tokio::spawn(async move {
            let mut request_buf = [0_u8; 1024];

            let read_result = socket.read(&mut request_buf).await;
            let request_len = match read_result {
                Ok(0) => return,
                Ok(n) => n,
                Err(_) => return,
            };

            let request = String::from_utf8_lossy(&request_buf[..request_len]);

            let is_metrics_path =
                request.starts_with("GET /metrics ") || request.starts_with("GET /metrics?");

            let (status, content_type, body) = if is_metrics_path {
                (
                    "200 OK",
                    "text/plain; version=0.0.4; charset=utf-8",
                    render_metrics(&state),
                )
            } else {
                (
                    "404 Not Found",
                    "text/plain; charset=utf-8",
                    "not found\n".to_string(),
                )
            };

            let response = format!(
                concat!(
                    "HTTP/1.1 {status}\r\n",
                    "Content-Type: {content_type}\r\n",
                    "Content-Length: {content_length}\r\n",
                    "Connection: close\r\n",
                    "\r\n",
                    "{body}"
                ),
                status = status,
                content_type = content_type,
                content_length = body.len(),
                body = body,
            );

            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.shutdown().await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_metrics_includes_spike_counter() {
        let state = PrometheusState::default();
        state.total_spikes.store(3, Ordering::Relaxed);

        let output = render_metrics(&state);

        assert!(output.contains("# HELP stutter_spikes_total"));
        assert!(output.contains("# TYPE stutter_spikes_total counter"));
        assert!(output.contains("stutter_spikes_total 3\n"));
    }

    #[test]
    fn render_metrics_includes_required_metrics() {
        let state = PrometheusState::default();

        let output = render_metrics(&state);

        for metric in [
            "stutter_spikes_total",
            "stutter_samples_total",
            "stutter_max_latency_ns",
            "stutter_latest_p99_ns",
            "stutter_active_targets",
            "stutter_event_stream_write_errors_total",
            "stutter_ebpf_drops_total",
            "stutter_autotune_phase",
            "stutter_autotune_mode",
            "stutter_autotune_active_experiment",
            "stutter_autotune_last_score",
            "stutter_autotune_candidate_score",
            "stutter_autotune_rollbacks_total",
            "stutter_autotune_actions_applied_total",
            "stutter_autotune_actions_blocked_total",
        ] {
            assert!(
                output.contains(metric),
                "missing metric {metric} in output:\n{output}"
            );
        }
    }

    #[test]
    fn render_metrics_includes_autotune_metric_help_and_types() {
        let state = PrometheusState::default();

        let output = render_metrics(&state);

        assert!(output.contains("# HELP stutter_autotune_phase"));
        assert!(output.contains("# TYPE stutter_autotune_phase gauge"));
        assert!(output.contains("# HELP stutter_autotune_mode"));
        assert!(output.contains("# TYPE stutter_autotune_mode gauge"));
        assert!(output.contains("# HELP stutter_autotune_active_experiment"));
        assert!(output.contains("# TYPE stutter_autotune_active_experiment gauge"));
        assert!(output.contains("# HELP stutter_autotune_last_score"));
        assert!(output.contains("# TYPE stutter_autotune_last_score gauge"));
        assert!(output.contains("# HELP stutter_autotune_candidate_score"));
        assert!(output.contains("# TYPE stutter_autotune_candidate_score gauge"));
        assert!(output.contains("# HELP stutter_autotune_rollbacks_total"));
        assert!(output.contains("# TYPE stutter_autotune_rollbacks_total counter"));
        assert!(output.contains("# HELP stutter_autotune_actions_applied_total"));
        assert!(output.contains("# TYPE stutter_autotune_actions_applied_total counter"));
        assert!(output.contains("# HELP stutter_autotune_actions_blocked_total"));
        assert!(output.contains("# TYPE stutter_autotune_actions_blocked_total counter"));
    }

    #[test]
    fn observe_latency_keeps_maximum() {
        let state = PrometheusState::default();

        state.observe_latency_ns(100);
        state.observe_latency_ns(50);
        state.observe_latency_ns(200);

        assert_eq!(state.max_latency_ns.load(Ordering::Relaxed), 200);
    }
}
