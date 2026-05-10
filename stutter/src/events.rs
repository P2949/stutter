use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

use log::{debug, info, warn};
use serde::Serialize;
use stutter_common::{
    BlockIoEvent, CpuFreqEvent, ExecEvent, IrqEvent, MigrationEvent, SchedulerEvent,
};

use crate::{
    artifacts::ArtifactKind,
    cli::Config,
    metrics::{self, format_latency},
    process_tree::{self, TaskClass},
    recorder::{self, IrqEventRecord, LiveRecorder, RecordingCounters},
    session::sinks::{MonitorEventSink, MonitorOutputSinks, RecorderSink},
    session_events::MonitorEvent,
    tasks::TaskTracker,
};

pub mod decode;
pub mod interpret;

pub fn handle_irq_record(record: &IrqEventRecord, recorder: &mut LiveRecorder) {
    let event = MonitorEvent::IrqEvent {
        event: Box::new(record.clone()),
    };
    if let Err(err) = RecorderSink::new(recorder).on_event(&event) {
        warn!("monitor_event_sink_failed err={err}");
    }
    log_irq_record(record);
}

pub fn handle_migration_event(
    event: &MigrationEvent,
    tasks: &mut TaskTracker,
    recorder: &mut LiveRecorder,
    cpu_to_pkg: &BTreeMap<u32, String>,
    started: Instant,
) {
    let elapsed_ms = started.elapsed().as_millis() as u64;

    if let Some(stats) = tasks.stats_by_task.get_mut(&event.tid) {
        stats.migration_count += 1;

        let from_pkg = cpu_to_pkg.get(&event.from_cpu);
        let to_pkg = cpu_to_pkg.get(&event.to_cpu);
        if let (Some(f), Some(t)) = (from_pkg, to_pkg)
            && f != t
        {
            stats.cross_numa_migrations += 1;
        }
    }

    let record = recorder::MigrationEventRecord {
        elapsed_ms,
        tid: event.tid,
        from_cpu: event.from_cpu,
        to_cpu: event.to_cpu,
        timestamp_ns: event.timestamp_ns,
    };

    let event = MonitorEvent::MigrationEvent {
        event: Box::new(record),
    };
    if let Err(err) = RecorderSink::new(recorder).on_event(&event) {
        warn!("monitor_event_sink_failed err={err}");
    }
}

pub fn handle_cpu_freq_event(event: &CpuFreqEvent, recorder: &mut LiveRecorder, started: Instant) {
    let elapsed_ms = started.elapsed().as_millis() as u64;

    let record = recorder::CpuFreqRecord {
        elapsed_ms,
        cpu: event.cpu,
        freq_khz: event.state,
        timestamp_ns: event.timestamp_ns,
    };

    let event = MonitorEvent::CpuFreqSample {
        event: Box::new(record),
    };
    if let Err(err) = RecorderSink::new(recorder).on_event(&event) {
        warn!("monitor_event_sink_failed err={err}");
    }
}

pub fn block_io_event_record(
    event: &BlockIoEvent,
    block_io_correlation_basis: &'static str,
    started: Instant,
) -> recorder::BlockIoRecord {
    let elapsed_ms = started.elapsed().as_millis() as u64;

    recorder::BlockIoRecord {
        elapsed_ms,
        tid: event.tid,
        correlation_basis: std::borrow::Cow::Borrowed(block_io_correlation_basis),
        dev: event.dev,
        nr_sector: event.nr_sector,
        sector: event.sector,
        duration_ns: event.duration_ns,
        timestamp_ns: event.timestamp_ns,
        rwbs: String::from_utf8_lossy(&event.rwbs)
            .trim_matches(char::from(0))
            .to_owned(),
    }
}

pub fn handle_block_io_record(record: &recorder::BlockIoRecord, recorder: &mut LiveRecorder) {
    let event = MonitorEvent::IoEvent {
        event: Box::new(record.clone()),
    };
    if let Err(err) = RecorderSink::new(recorder).on_event(&event) {
        warn!("monitor_event_sink_failed err={err}");
    }
}

pub fn handle_exec_event(item: &[u8], tasks: &mut TaskTracker) {
    let Some(event) = read_event_unaligned::<ExecEvent>(item) else {
        warn!("short_exec_event len={}", item.len());
        return;
    };
    let comm = metrics::comm_to_string(&event.comm);
    tasks.cache.invalidate(event.pid);
    tasks.cache.invalidate(event.tid);

    info!(
        "process_exec pid={} tid={} comm={}",
        event.pid, event.tid, comm
    );

    if let Some(stats) = tasks.stats_by_task.get_mut(&event.tid) {
        stats.comm = comm.clone();
        stats.class = process_tree::classify_task(&comm, &comm, "");
    }

    if let Some(info) = tasks.active_targets.get_mut(&event.tid) {
        info.comm = comm.clone();
        info.class = process_tree::classify_task(&comm, &comm, "");
    }
}

/// Pushes an event to an NDJSON stream via the registry.
pub fn push_artifact_event<T: Serialize, F>(
    recorder: &mut LiveRecorder,
    kind: ArtifactKind,
    value: &T,
    stream_name: &str,
    mut success_fn: F,
) where
    F: FnMut(&mut RecordingCounters),
{
    match recorder.streams.push(kind, value) {
        Ok(true) => success_fn(&mut recorder.counters),
        Ok(false) => {}
        Err(err) => {
            warn!("ndjson_write_failed stream={stream_name} err={err:#}");
            recorder
                .counters
                .record_stream_write_error(stream_name, err);
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn handle_event(
    event: &SchedulerEvent,
    config: &Config,
    started: Instant,
    tasks: &mut TaskTracker,
    monotonic_start_ns: Option<u64>,
    recorder: &mut LiveRecorder,
    alert_sender: Option<&tokio::sync::mpsc::Sender<AlertPayload>>,
    scx_ops: Option<&str>,
    scx_state: Option<&str>,
    scx_enable_seq: Option<&str>,
) -> Option<recorder::SpikeEvent> {
    let spike_config = interpret::SpikeConfig {
        spike_threshold_ns: config.spike_threshold_ns,
        alert_threshold_ns: config.alert_threshold_ns,
        verbose: config.verbose,
        cgroupv2_active: config.cgroupv2.is_some(),
    };

    let update = interpret::interpret_scheduler_event(
        event,
        &spike_config,
        started,
        tasks,
        monotonic_start_ns,
        scx_ops,
        scx_state,
        scx_enable_seq,
    );

    let mut sinks = MonitorOutputSinks::new(config, recorder, alert_sender);
    for event in &update.events {
        if let Err(err) = sinks.dispatch(event) {
            warn!("monitor_event_sink_failed err={err}");
        }
    }

    update.spike_event
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct AlertPayload {
    pub title: String,
    pub message: String,
    pub task: u32,
    pub active: bool,
    pub class: TaskClass,
    pub comm: String,
    pub process_pid: Option<u32>,
    pub process_comm: String,
    pub latency_ns: u64,
    pub latency_ms: u64,
    pub cpu: u32,
    pub prio: i32,
    pub wakeup_ns: u64,
    pub switch_ns: u64,
    pub elapsed_ms: u64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub scx_ops: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub scx_state: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub scx_enable_seq: Option<String>,
}

impl AlertPayload {
    pub fn from_task_stats(
        stats: &metrics::TaskStats,
        event: &SchedulerEvent,
        elapsed_ms: u64,
        scx_ops: Option<&str>,
        scx_state: Option<&str>,
        scx_enable_seq: Option<&str>,
    ) -> Self {
        let latency_ms = event.latency_ns / 1_000_000;
        let title = "stutter latency alert".to_owned();
        let message = format!(
            "task={} comm={} latency={} cpu={} process_pid={:?} process_comm={}",
            event.tid,
            stats.comm,
            format_latency(event.latency_ns),
            event.cpu,
            stats.process_pid,
            stats.process_comm
        );

        Self {
            title,
            message,
            task: event.tid,
            active: stats.active,
            class: stats.class,
            comm: stats.comm.clone(),
            process_pid: stats.process_pid,
            process_comm: stats.process_comm.to_string(),
            latency_ns: event.latency_ns,
            latency_ms,
            cpu: event.cpu,
            prio: event.prio,
            wakeup_ns: event.wakeup_ns,
            switch_ns: event.switch_ns,
            elapsed_ms,
            scx_ops: scx_ops.map(str::to_owned),
            scx_state: scx_state.map(str::to_owned),
            scx_enable_seq: scx_enable_seq.map(str::to_owned),
        }
    }
}

/// Sends a desktop notification using `notify-send`.
///
/// This remains a best-effort local desktop integration. It intentionally
/// uses an external command and may add system noise, so alert failures are
/// logged by the caller and do not stop monitoring.
pub async fn send_desktop_alert(payload: &AlertPayload) -> Result<(), String> {
    let mut child = tokio::process::Command::new("notify-send")
        .args([
            "--urgency=critical",
            payload.title.as_str(),
            payload.message.as_str(),
        ])
        .spawn()
        .map_err(|err| format!("failed to spawn notify-send: {err}"))?;

    let status = tokio::time::timeout(Duration::from_secs(10), child.wait())
        .await
        .map_err(|_| "notify-send timed out after 10 seconds".to_owned())?
        .map_err(|err| format!("failed to wait for notify-send: {err}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("notify-send exited with {status}"))
    }
}

fn validate_webhook_url(url: &str) -> Result<url::Url, String> {
    let parsed = url::Url::parse(url).map_err(|err| format!("invalid webhook URL: {err}"))?;

    match parsed.scheme() {
        "http" | "https" => Ok(parsed),
        other => Err(format!(
            "unsupported webhook URL scheme `{other}`; only http and https are allowed"
        )),
    }
}

pub async fn send_webhook_alert_with_client(
    client: &reqwest::Client,
    url: &str,
    payload: &AlertPayload,
) -> Result<(), String> {
    let parsed_url = validate_webhook_url(url)?;
    let response = client
        .post(parsed_url)
        .json(payload)
        .send()
        .await
        .map_err(|err| format!("failed to send webhook alert: {err}"))?;

    let status = response.status();
    if status.is_success() {
        Ok(())
    } else {
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<failed to read response body>".to_owned());

        Err(format!(
            "webhook alert failed with HTTP status {status}: {body}"
        ))
    }
}

#[allow(dead_code)]
pub async fn send_webhook_alert(url: &str, payload: &AlertPayload) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|err| format!("failed to build webhook HTTP client: {err}"))?;

    send_webhook_alert_with_client(&client, url, payload).await
}

pub fn irq_event_record(monotonic_start_ns: Option<u64>, event: &IrqEvent) -> IrqEventRecord {
    IrqEventRecord {
        elapsed_ms: monotonic_start_ns
            .map(|start| event.enter_ns.saturating_sub(start) / 1_000_000),
        cpu: event.cpu,
        irq: event.irq,
        enter_ns: event.enter_ns,
        exit_ns: event.exit_ns,
        duration_ns: event.duration_ns,
    }
}

pub fn log_irq_record(record: &IrqEventRecord) {
    debug!(
        "irq_event cpu={} irq={} latency={}",
        record.cpu,
        record.irq,
        format_latency(record.duration_ns)
    );
}

#[allow(dead_code)]
pub fn log_irq_event(event: &IrqEvent) {
    debug!(
        "irq_event cpu={} irq={} latency={}",
        event.cpu,
        event.irq,
        format_latency(event.duration_ns)
    );
}

pub fn read_event_unaligned<T: aya::Pod + Copy>(data: &[u8]) -> Option<T> {
    if data.len() < std::mem::size_of::<T>() {
        return None;
    }

    Some(unsafe { (data.as_ptr() as *const T).read_unaligned() })
}

const IMMEDIATE_CAUSE_TAG_PRIORITY: &[&str] = &[
    "major_page_fault",
    "minor_page_fault",
    "runqueue_contention",
    "migration_or_cpu_mismatch",
    "monitored_wakeup_backlog",
];

const RESERVED_CROSS_SIGNAL_TAG_PRIORITY: &[&str] = &[
    // Reserved for future/cross-signal diagnosis. These tags are not emitted by
    // immediate_cause_tags() today, but primary_from_tags() accepts them so
    // higher-level diagnosis code can use the same primary-cause selection path.
    "cpu_frequency",
    "irq_interference",
    "gpu_frame_pressure",
    "block_io",
];

const PRIMARY_CAUSE_PRIORITY: &[&str] = &[
    "major_page_fault",
    "minor_page_fault",
    "runqueue_contention",
    // Reserved for future/cross-signal diagnosis. These tags are not emitted by
    // immediate_cause_tags() today, but primary_from_tags() intentionally accepts
    // them so report/diagnosis code can share one priority rule.
    "cpu_frequency",
    "irq_interference",
    "gpu_frame_pressure",
    "block_io",
    "migration_or_cpu_mismatch",
    "monitored_wakeup_backlog",
];

// Generates only immediate per-scheduler-event tags from fields available on
// SchedulerEvent/TaskStats. Cross-signal tags such as block_io, irq_interference,
// gpu_frame_pressure, and cpu_frequency are intentionally not emitted here; they
// belong to higher-level correlation/diagnosis code.
pub(crate) fn immediate_cause_tags(
    event: &SchedulerEvent,
    _stats: &metrics::TaskStats,
    fault_deltas: (u64, u64),
) -> Vec<String> {
    let mut tags = Vec::with_capacity(IMMEDIATE_CAUSE_TAG_PRIORITY.len());

    if event.observed_runnable_depth >= 4 {
        tags.push("runqueue_contention".to_string());
    }

    if event.target_pending_wakeups > 2 {
        tags.push("monitored_wakeup_backlog".to_string());
    }

    if fault_deltas.0 > 0 {
        tags.push("major_page_fault".to_string());
    }
    if fault_deltas.1 > 0 {
        tags.push("minor_page_fault".to_string());
    }

    if event.wakeup_target_cpu != event.cpu {
        tags.push("migration_or_cpu_mismatch".to_string());
    }

    tags
}

pub(crate) fn primary_from_tags(tags: &[String]) -> Option<String> {
    debug_assert!(
        IMMEDIATE_CAUSE_TAG_PRIORITY
            .iter()
            .all(|candidate| PRIMARY_CAUSE_PRIORITY.contains(candidate))
    );
    debug_assert!(
        RESERVED_CROSS_SIGNAL_TAG_PRIORITY
            .iter()
            .all(|candidate| PRIMARY_CAUSE_PRIORITY.contains(candidate))
    );

    PRIMARY_CAUSE_PRIORITY
        .iter()
        .find(|candidate| tags.iter().any(|tag| tag == **candidate))
        .map(|cause| cause.to_string())
}

#[cfg(test)]
mod tests {
    use stutter_common::{EVENT_BLOCK_IO, EVENT_IRQ_LATENCY, EVENT_RUNNABLE_LATENCY, IrqEvent};

    use super::*;

    #[test]
    fn validate_webhook_url_allows_http_and_https() {
        assert!(validate_webhook_url("http://example.com/hook").is_ok());
        assert!(validate_webhook_url("https://example.com/hook").is_ok());
    }

    #[test]
    fn validate_webhook_url_rejects_non_http_schemes() {
        let err = validate_webhook_url("file:///tmp/hook").unwrap_err();
        assert!(err.contains("only http and https are allowed"));

        let err = validate_webhook_url("ftp://example.com/hook").unwrap_err();
        assert!(err.contains("only http and https are allowed"));
    }

    #[test]
    fn validate_webhook_url_rejects_relative_urls() {
        assert!(validate_webhook_url("example.com/hook").is_err());
        assert!(validate_webhook_url("/local/path").is_err());
    }

    #[test]
    fn irq_event_record_preserves_event_fields() {
        let event = IrqEvent {
            kind: EVENT_IRQ_LATENCY,
            irq: 44,
            cpu: 3,
            enter_ns: 1_000_000,
            exit_ns: 1_250_000,
            duration_ns: 250_000,
        };

        let record = irq_event_record(Some(500_000), &event);

        assert_eq!(record.elapsed_ms, Some(0));
        assert_eq!(record.cpu, 3);
        assert_eq!(record.irq, 44);
        assert_eq!(record.enter_ns, 1_000_000);
        assert_eq!(record.exit_ns, 1_250_000);
        assert_eq!(record.duration_ns, 250_000);
    }

    #[test]
    fn irq_event_record_without_monotonic_start_has_no_elapsed_ms() {
        let event = IrqEvent {
            kind: EVENT_IRQ_LATENCY,
            irq: 44,
            cpu: 3,
            enter_ns: 1_000_000,
            exit_ns: 1_250_000,
            duration_ns: 250_000,
        };

        let record = irq_event_record(None, &event);

        assert_eq!(record.elapsed_ms, None);
    }

    #[test]
    fn test_unaligned_event_decoding() {
        let event = SchedulerEvent {
            kind: EVENT_RUNNABLE_LATENCY,
            tid: 123,
            cpu: 1,
            wakeup_target_cpu: 1,
            prio: 120,
            waker_tid: 0,
            target_pending_wakeups: 0,
            observed_runnable_depth: 0,
            maj_flt: 0,
            min_flt: 0,
            wakeup_ns: 2000,
            switch_ns: 3000,
            latency_ns: 1000,
            comm: [0; 16],
            switch_prev_pid: 0,
            switch_prev_state: 0,
        };

        let bytes = unsafe {
            std::slice::from_raw_parts(
                &event as *const SchedulerEvent as *const u8,
                std::mem::size_of::<SchedulerEvent>(),
            )
        };

        // Build a deliberately misaligned buffer
        let mut misaligned = vec![0u8];
        misaligned.extend_from_slice(bytes);

        let decoded = read_event_unaligned::<SchedulerEvent>(&misaligned[1..]).unwrap();
        assert_eq!(decoded.kind, EVENT_RUNNABLE_LATENCY);
        assert_eq!(decoded.tid, 123);
    }

    #[test]
    fn test_immediate_cause_tags() {
        let mut event = SchedulerEvent {
            kind: EVENT_RUNNABLE_LATENCY,
            tid: 123,
            cpu: 1,
            wakeup_target_cpu: 1,
            prio: 120,
            waker_tid: 0,
            target_pending_wakeups: 0,
            observed_runnable_depth: 0,
            maj_flt: 0,
            min_flt: 0,
            wakeup_ns: 0,
            switch_ns: 0,
            latency_ns: 1000,
            comm: [0; 16],
            switch_prev_pid: 0,
            switch_prev_state: 0,
        };
        let stats = metrics::TaskStats::new(123, "test".to_string(), 0);

        // No tags
        let tags = immediate_cause_tags(&event, &stats, (0, 0));
        assert!(tags.is_empty());

        // Major fault
        let tags = immediate_cause_tags(&event, &stats, (1, 0));
        assert!(tags.contains(&"major_page_fault".to_string()));

        // Minor fault
        let tags = immediate_cause_tags(&event, &stats, (0, 1));
        assert!(tags.contains(&"minor_page_fault".to_string()));

        // Wakeup backlog
        event.target_pending_wakeups = 5;
        let tags = immediate_cause_tags(&event, &stats, (0, 0));
        assert!(tags.contains(&"monitored_wakeup_backlog".to_string()));

        // Migration
        event.target_pending_wakeups = 0;
        event.wakeup_target_cpu = 2;
        let tags = immediate_cause_tags(&event, &stats, (0, 0));
        assert!(tags.contains(&"migration_or_cpu_mismatch".to_string()));

        // Both major and minor faults
        let tags = immediate_cause_tags(&event, &stats, (1, 1));
        assert!(tags.contains(&"major_page_fault".to_string()));
        assert!(tags.contains(&"minor_page_fault".to_string()));
    }

    #[test]
    fn test_primary_from_tags() {
        let tags = vec![
            "migration_or_cpu_mismatch".to_string(),
            "major_page_fault".to_string(),
        ];
        // major_page_fault has higher priority
        assert_eq!(
            primary_from_tags(&tags),
            Some("major_page_fault".to_string())
        );

        let tags = vec![
            "monitored_wakeup_backlog".to_string(),
            "migration_or_cpu_mismatch".to_string(),
        ];
        // migration_or_cpu_mismatch has higher priority
        assert_eq!(
            primary_from_tags(&tags),
            Some("migration_or_cpu_mismatch".to_string())
        );

        assert_eq!(primary_from_tags(&[]), None);
    }

    #[test]
    fn test_primary_from_tags_priority() {
        let tags = vec![
            "minor_page_fault".to_string(),
            "major_page_fault".to_string(),
        ];
        // major_page_fault has higher priority
        assert_eq!(
            primary_from_tags(&tags),
            Some("major_page_fault".to_string())
        );

        let tags = vec!["minor_page_fault".to_string()];
        assert_eq!(
            primary_from_tags(&tags),
            Some("minor_page_fault".to_string())
        );
    }

    #[test]
    fn primary_from_tags_accepts_reserved_cross_signal_tags() {
        let tags = vec!["block_io".to_string()];

        assert_eq!(primary_from_tags(&tags), Some("block_io".to_string()));
    }

    #[test]
    fn primary_from_tags_keeps_major_fault_above_reserved_cross_signal_tags() {
        let tags = vec!["block_io".to_string(), "major_page_fault".to_string()];

        assert_eq!(
            primary_from_tags(&tags),
            Some("major_page_fault".to_string())
        );
    }

    #[test]
    fn spike_record_is_created_with_cause_tags_without_late_patch() {
        let event = SchedulerEvent {
            kind: EVENT_RUNNABLE_LATENCY,
            tid: 123,
            cpu: 1,
            wakeup_target_cpu: 1,
            prio: 120,
            waker_tid: 0,
            target_pending_wakeups: 0,
            observed_runnable_depth: 4,
            maj_flt: 0,
            min_flt: 0,
            wakeup_ns: 10,
            switch_ns: 20,
            latency_ns: 2_000_000,
            comm: [0; 16],
            switch_prev_pid: 0,
            switch_prev_state: 0,
        };

        let mut stats = metrics::TaskStats::new(123, "test".to_string(), 0);

        let fault_deltas = (
            event.maj_flt.saturating_sub(stats.last_spike_major_faults),
            event.min_flt.saturating_sub(stats.last_spike_minor_faults),
        );
        let cause_tags = immediate_cause_tags(&event, &stats, fault_deltas);
        let primary_cause = primary_from_tags(&cause_tags);

        stats.record(
            &event,
            1_000_000,
            0,
            Some(metrics::SpikeRecordDiagnostics {
                scx_ops: Some("ops".to_string()),
                scx_state: Some("enabled".to_string()),
                scx_enable_seq: Some("42".to_string()),
                cause_tags: cause_tags.clone(),
                primary_cause: primary_cause.clone(),
            }),
        );

        assert_eq!(stats.top_spikes.len(), 1);
        let spike = &stats.top_spikes[0];

        assert_eq!(spike.cause_tags, cause_tags);
        assert_eq!(spike.primary_cause, primary_cause);
        assert_eq!(spike.scx_ops.as_deref(), Some("ops"));
        assert_eq!(spike.scx_state.as_deref(), Some("enabled"));
        assert_eq!(spike.scx_enable_seq.as_deref(), Some("42"));
    }

    #[test]
    fn block_io_event_record_preserves_event_fields() {
        let event = BlockIoEvent {
            kind: EVENT_BLOCK_IO,
            tid: 42,
            dev: 123,
            nr_sector: 8,
            sector: 999,
            duration_ns: 55_000,
            timestamp_ns: 777,
            rwbs: *b"R\0\0\0\0\0\0\0",
        };

        let started = std::time::Instant::now();
        let record = block_io_event_record(&event, "dev+sector", started);

        assert_eq!(record.tid, 42);
        assert_eq!(record.correlation_basis.as_ref(), "dev+sector");
        assert_eq!(record.dev, 123);
        assert_eq!(record.nr_sector, 8);
        assert_eq!(record.sector, 999);
        assert_eq!(record.duration_ns, 55_000);
        assert_eq!(record.timestamp_ns, 777);
        assert_eq!(record.rwbs, "R");
        assert!(matches!(
            record.correlation_basis,
            std::borrow::Cow::Borrowed("dev+sector")
        ));
    }
}
