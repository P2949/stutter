#[cfg(any(feature = "otel", test))]
use std::time::Duration;
use std::time::SystemTime;

use anyhow::Result;

#[cfg(feature = "otel")]
mod enabled {
    use std::sync::{Arc, atomic::AtomicU64};

    use opentelemetry::{
        KeyValue, global,
        trace::{Span, Tracer, TracerProvider},
    };
    use opentelemetry_otlp::WithExportConfig;
    use opentelemetry_sdk::{Resource, runtime, trace as sdktrace};
    use tokio::sync::mpsc;

    use super::*;
    use crate::recorder::SpikeEvent;

    #[derive(Debug, Clone)]
    pub struct OtelConfig {
        pub endpoint: String,
        pub service_name: String,
        pub started_at: SystemTime,
        pub monotonic_start_ns: u64,
    }

    #[derive(Debug)]
    pub struct OtelExporterHandle {
        pub tx: mpsc::Sender<OtelSpike>,
        pub dropped: Arc<AtomicU64>,
    }

    #[derive(Debug, Clone)]
    pub struct OtelSpike {
        pub wakeup_ns: u64,
        pub switch_ns: u64,
        pub task_tid: u32,
        pub task_comm: String,
        pub task_class: String,
        pub process_pid: u32,
        pub process_comm: String,
        pub cpu: u32,
        pub wakeup_target_cpu: u32,
        pub latency_ns: u64,
        pub prio: i32,
        pub primary_cause: Option<String>,
    }

    impl From<&SpikeEvent> for OtelSpike {
        fn from(spike: &SpikeEvent) -> Self {
            Self {
                wakeup_ns: spike.wakeup_ns,
                switch_ns: spike.switch_ns,
                task_tid: spike.task,
                task_comm: spike.comm.clone(),
                task_class: spike.class.to_string(),
                process_pid: spike.process_pid.unwrap_or(0),
                process_comm: spike.process_comm.to_string(),
                cpu: spike.cpu,
                wakeup_target_cpu: spike.wakeup_target_cpu,
                latency_ns: spike.latency_ns,
                prio: spike.prio,
                primary_cause: spike.primary_cause.clone(),
            }
        }
    }

    pub fn spawn_exporter(config: OtelConfig) -> Result<OtelExporterHandle> {
        let (tx, mut rx) = mpsc::channel::<OtelSpike>(4096);
        let dropped = Arc::new(AtomicU64::new(0));

        let provider = opentelemetry_otlp::new_pipeline()
            .tracing()
            .with_exporter(
                opentelemetry_otlp::new_exporter()
                    .tonic()
                    .with_endpoint(config.endpoint.clone()),
            )
            .with_trace_config(
                sdktrace::Config::default().with_resource(Resource::new(vec![KeyValue::new(
                    "service.name",
                    config.service_name.clone(),
                )])),
            )
            .install_batch(runtime::Tokio)?;

        let tracer = provider.tracer("stutter");

        tokio::spawn(async move {
            while let Some(spike) = rx.recv().await {
                export_spike_span(&tracer, &config, spike);
            }
            global::shutdown_tracer_provider();
        });

        Ok(OtelExporterHandle { tx, dropped })
    }

    fn export_spike_span<T: Tracer>(tracer: &T, config: &OtelConfig, spike: OtelSpike) {
        let start_time = monotonic_ns_to_system_time(
            config.started_at,
            config.monotonic_start_ns,
            spike.wakeup_ns,
        );

        let end_time = monotonic_ns_to_system_time(
            config.started_at,
            config.monotonic_start_ns,
            spike.switch_ns,
        );

        let mut span = tracer
            .span_builder("stutter.scheduler_spike")
            .with_start_time(start_time)
            .start(tracer);

        span.set_attribute(KeyValue::new("task.tid", spike.task_tid as i64));
        span.set_attribute(KeyValue::new("task.comm", spike.task_comm));
        span.set_attribute(KeyValue::new("task.class", spike.task_class));
        span.set_attribute(KeyValue::new("task.process_pid", spike.process_pid as i64));
        span.set_attribute(KeyValue::new("task.process_comm", spike.process_comm));
        span.set_attribute(KeyValue::new("scheduler.cpu", spike.cpu as i64));
        span.set_attribute(KeyValue::new(
            "scheduler.wakeup_target_cpu",
            spike.wakeup_target_cpu as i64,
        ));
        span.set_attribute(KeyValue::new(
            "scheduler.latency_ns",
            i64::try_from(spike.latency_ns).unwrap_or(i64::MAX),
        ));
        span.set_attribute(KeyValue::new("scheduler.prio", spike.prio as i64));

        if let Some(primary_cause) = spike.primary_cause {
            span.set_attribute(KeyValue::new("stutter.primary_cause", primary_cause));
        }

        span.end_with_timestamp(end_time);
    }
}

#[cfg(not(feature = "otel"))]
mod disabled {
    use std::sync::{Arc, atomic::AtomicU64};

    use tokio::sync::mpsc;

    use super::*;
    use crate::recorder::SpikeEvent;

    #[derive(Debug, Clone)]
    pub struct OtelConfig {
        pub endpoint: String,
        pub service_name: String,
        pub started_at: SystemTime,
        pub monotonic_start_ns: u64,
    }

    #[derive(Debug)]
    pub struct OtelExporterHandle {
        pub tx: mpsc::Sender<OtelSpike>,
        pub dropped: Arc<AtomicU64>,
    }

    #[derive(Debug, Clone)]
    pub struct OtelSpike;

    impl From<&SpikeEvent> for OtelSpike {
        fn from(_spike: &SpikeEvent) -> Self {
            Self
        }
    }

    pub fn spawn_exporter(config: OtelConfig) -> Result<OtelExporterHandle> {
        let OtelConfig {
            endpoint,
            service_name,
            started_at,
            monotonic_start_ns,
        } = config;

        anyhow::bail!(
            "OpenTelemetry support was not compiled in. Rebuild with --features otel. \
requested endpoint={endpoint:?} service_name={service_name:?} \
started_at={started_at:?} monotonic_start_ns={monotonic_start_ns}"
        );
    }
}

#[cfg(not(feature = "otel"))]
pub use disabled::*;
#[cfg(feature = "otel")]
pub use enabled::*;

#[cfg(any(feature = "otel", test))]
pub fn monotonic_ns_to_system_time(
    started_at: SystemTime,
    monotonic_start_ns: u64,
    event_ns: u64,
) -> SystemTime {
    if event_ns >= monotonic_start_ns {
        started_at + Duration::from_nanos(event_ns - monotonic_start_ns)
    } else {
        started_at
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::*;

    #[test]
    fn monotonic_ns_to_system_time_maps_forward_delta() {
        let started_at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);

        let mapped = monotonic_ns_to_system_time(started_at, 10_000, 15_000);

        assert_eq!(mapped, started_at + Duration::from_nanos(5_000));
    }

    #[test]
    fn monotonic_ns_to_system_time_clamps_events_before_start() {
        let started_at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);

        let mapped = monotonic_ns_to_system_time(started_at, 10_000, 5_000);

        assert_eq!(mapped, started_at);
    }
}
