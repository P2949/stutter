use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

use log::{debug, info, warn};
use serde::Serialize;
use stutter_common::{
    BlockIoEvent, CpuFreqEvent, EVENT_RUNNABLE_LATENCY, ExecEvent, IrqEvent, MigrationEvent,
    SchedulerEvent,
};

use crate::{
    cli::Config,
    metrics::{self, format_latency, print_event},
    process_tree::{self, TaskClass},
    recorder::{self, IrqEventRecord, JsonArrayWriter, LiveRecorder},
    tasks::{TaskTracker, should_replace_unknown_comm},
};

pub fn handle_irq_event(
    event: &IrqEvent,
    recorder: &mut LiveRecorder,
    monotonic_start_ns: Option<u64>,
) {
    let record = irq_event_record(monotonic_start_ns, event);
    if let Some(writer) = recorder.irq_event_writer.as_mut() {
        push_ndjson_event(
            writer,
            &record,
            &mut recorder.irq_event_count,
            &mut recorder.event_stream_write_errors,
            &mut recorder.first_event_stream_write_error,
            "irq_events",
        );
    }
    log_irq_event(event);
}

pub fn handle_migration_event(
    event: &MigrationEvent,
    tasks: &mut TaskTracker,
    recorder: &mut LiveRecorder,
    cpu_to_pkg: &BTreeMap<u32, String>,
    started: Instant,
) {
    let elapsed_ms = started.elapsed().as_millis();

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

    if let Some(writer) = recorder.migration_event_writer.as_mut() {
        let record = recorder::MigrationEventRecord {
            elapsed_ms,
            tid: event.tid,
            from_cpu: event.from_cpu,
            to_cpu: event.to_cpu,
            timestamp_ns: event.timestamp_ns,
        };
        push_ndjson_event(
            writer,
            &record,
            &mut recorder.migration_event_count,
            &mut recorder.event_stream_write_errors,
            &mut recorder.first_event_stream_write_error,
            "migration_events",
        );
    }
}

pub fn handle_cpu_freq_event(event: &CpuFreqEvent, recorder: &mut LiveRecorder, started: Instant) {
    let elapsed_ms = started.elapsed().as_millis();

    if let Some(writer) = recorder.cpu_freq_sample_writer.as_mut() {
        let record = recorder::CpuFreqRecord {
            elapsed_ms,
            cpu: event.cpu,
            freq_khz: event.state,
            timestamp_ns: event.timestamp_ns,
        };
        push_ndjson_event(
            writer,
            &record,
            &mut recorder.cpu_freq_sample_count,
            &mut recorder.event_stream_write_errors,
            &mut recorder.first_event_stream_write_error,
            "cpu_freq_samples",
        );
    }
}

pub fn handle_block_io_event(
    event: &BlockIoEvent,
    recorder: &mut LiveRecorder,
    block_io_correlation_basis: &str,
    started: Instant,
) {
    let elapsed_ms = started.elapsed().as_millis();

    if let Some(writer) = recorder.block_io_event_writer.as_mut() {
        let record = recorder::BlockIoRecord {
            elapsed_ms,
            tid: event.tid,
            correlation_basis: block_io_correlation_basis.to_owned(),
            dev: event.dev,
            nr_sector: event.nr_sector,
            sector: event.sector,
            duration_ns: event.duration_ns,
            timestamp_ns: event.timestamp_ns,
            rwbs: String::from_utf8_lossy(&event.rwbs)
                .trim_matches(char::from(0))
                .to_owned(),
        };
        push_ndjson_event(
            writer,
            &record,
            &mut recorder.block_io_event_count,
            &mut recorder.event_stream_write_errors,
            &mut recorder.first_event_stream_write_error,
            "io_events",
        );
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

/// Pushes an event to an NDJSON stream.
pub fn push_ndjson_event<T: Serialize>(
    writer: &mut JsonArrayWriter,
    value: &T,
    count: &mut u64,
    error_count: &mut u64,
    first_error: &mut Option<String>,
    stream_name: &str,
) {
    match writer.push(value) {
        Ok(()) => *count += 1,
        Err(err) => {
            warn!("ndjson_write_failed stream={stream_name} err={err:#}");
            *error_count += 1;
            if first_error.is_none() {
                *first_error = Some(format!("{stream_name}: {err:#}"));
            }
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
    scx_ops: Option<String>,
    scx_state: Option<String>,
    scx_enable_seq: Option<String>,
) -> Option<recorder::SpikeEvent> {
    debug_assert_eq!(event.kind, EVENT_RUNNABLE_LATENCY);

    let comm = metrics::comm_to_string(&event.comm);
    let elapsed_ms = started.elapsed().as_millis();

    let task_info = tasks
        .active_targets
        .get(&event.pid)
        .or_else(|| tasks.known_targets.get(&event.pid));

    let stats = tasks
        .stats_by_task
        .entry(event.pid)
        .or_insert_with(|| metrics::TaskStats::new(event.pid, comm.clone(), elapsed_ms));

    if should_replace_unknown_comm(&stats.comm, &comm) {
        stats.comm = comm.clone();
    }

    if let Some(task_info) = task_info {
        stats.apply_task_info(task_info);
        stats.active = tasks.active_targets.contains_key(&event.pid);
    } else if config.cgroupv2.is_some() {
        stats.active = true;
    }

    let fault_deltas = stats.record(
        event,
        config.spike_threshold_ns,
        elapsed_ms,
        scx_ops.clone(),
        scx_state.clone(),
        scx_enable_seq.clone(),
    );

    if let Some(state) = recorder.prometheus_state.as_ref() {
        state.inc_samples(1);
        state.observe_latency_ns(event.latency_ns);
    }

    let mut spike_ret = None;
    if event.latency_ns >= config.spike_threshold_ns {
        let cause_tags = immediate_cause_tags(event, stats, fault_deltas);
        let primary_cause = primary_from_tags(&cause_tags);

        // Update the internal top spikes record so it persists into the session summary too
        if let Some(spike) = stats
            .top_spikes
            .iter_mut()
            .find(|s| s.switch_ns == event.switch_ns && s.cpu == event.cpu)
        {
            spike.cause_tags = cause_tags.clone();
            spike.primary_cause = primary_cause.clone();
        }

        let spike_event = recorder::SpikeEvent::from_task_stats(
            monotonic_start_ns,
            stats,
            event,
            fault_deltas,
            recorder::SpikeDiagnosticContext {
                scx_ops: scx_ops.clone(),
                scx_state: scx_state.clone(),
                scx_enable_seq: scx_enable_seq.clone(),
                cause_tags,
                primary_cause,
            },
        );
        spike_ret = Some(spike_event.clone());

        if let Some(state) = recorder.prometheus_state.as_ref() {
            state.inc_spikes(1);
        }

        if let Some(writer) = recorder.spike_event_writer.as_mut() {
            push_ndjson_event(
                writer,
                &spike_event,
                &mut recorder.spike_event_count,
                &mut recorder.event_stream_write_errors,
                &mut recorder.first_event_stream_write_error,
                "spike_events",
            );
        } else if let Some(spike_events) = recorder.spike_events.as_mut() {
            match spike_events.push(spike_event.clone()) {
                recorder::SpikePushResult::Stored => {}
                recorder::SpikePushResult::Dropped => recorder.spike_events_dropped_count += 1,
            }
        }

        if let Some(stream) = recorder.stdout_spike_stream.as_mut()
            && let Err(err) = stream.push(&spike_event)
        {
            warn!("json_stream_write_failed err={err:#}");
            recorder.stdout_spike_stream_errors += 1;
        }

        if !config.json_stream {
            if config.verbose {
                print_event(event, &comm, "sample");
            } else {
                print_event(event, &comm, "spike");
            }
        }
    } else if config.verbose && !config.json_stream {
        print_event(event, &comm, "sample");
    }

    if let Some(threshold) = config.alert_threshold_ns
        && event.latency_ns >= threshold
        && let Some(sender) = alert_sender
    {
        let alert_payload = AlertPayload::from_task_stats(
            stats,
            event,
            elapsed_ms,
            scx_ops.clone(),
            scx_state.clone(),
            scx_enable_seq.clone(),
        );
        if let Err(err) = sender.try_send(alert_payload) {
            match err {
                tokio::sync::mpsc::error::TrySendError::Full(_) => {
                    warn!("alert_channel_full_dropping_alert");
                    recorder.alert_events_dropped_count += 1;
                }
                tokio::sync::mpsc::error::TrySendError::Closed(_) => {
                    warn!("alert_channel_closed");
                    recorder.alert_channel_closed_count += 1;
                }
            }
        }
    }

    spike_ret
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
    pub elapsed_ms: u128,

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
        elapsed_ms: u128,
        scx_ops: Option<String>,
        scx_state: Option<String>,
        scx_enable_seq: Option<String>,
    ) -> Self {
        let latency_ms = event.latency_ns / 1_000_000;
        let title = "stutter latency alert".to_owned();
        let message = format!(
            "task={} comm={} latency={} cpu={} process_pid={:?} process_comm={}",
            event.pid,
            stats.comm,
            format_latency(event.latency_ns),
            event.cpu,
            stats.process_pid,
            stats.process_comm
        );

        Self {
            title,
            message,
            task: event.pid,
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
            scx_ops,
            scx_state,
            scx_enable_seq,
        }
    }
}

/// Sends a desktop notification using the `notify-send` command.
///
/// NOTE: This spawns an external process which can add system noise.
/// TODO: Replace with a native implementation using `zbus` or `freedesktop-notifications`.
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

/// Sends a webhook alert using the `curl` command.
///
/// NOTE: This spawns an external process which can add system noise.
/// TODO: Replace with a native implementation using `reqwest` or a `tiny-hyper` client.
pub async fn send_webhook_alert(url: &str, payload: &AlertPayload) -> Result<(), String> {
    let body = serde_json::to_string(payload)
        .map_err(|err| format!("failed to serialize alert payload: {err}"))?;
    let mut child = tokio::process::Command::new("curl")
        .args([
            "-fsS",
            "--max-time",
            "10",
            "-H",
            "Content-Type: application/json",
            "-d",
            &body,
            url,
        ])
        .spawn()
        .map_err(|err| format!("failed to spawn curl: {err}"))?;

    let status = tokio::time::timeout(Duration::from_secs(12), child.wait())
        .await
        .map_err(|_| "curl timed out after 12 seconds".to_owned())?
        .map_err(|err| format!("failed to wait for curl: {err}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("curl exited with {status}"))
    }
}

pub fn irq_event_record(monotonic_start_ns: Option<u64>, event: &IrqEvent) -> IrqEventRecord {
    IrqEventRecord {
        elapsed_ms: monotonic_start_ns
            .map(|start| (event.enter_ns.saturating_sub(start)) as u128 / 1_000_000),
        cpu: event.cpu,
        irq: event.irq,
        enter_ns: event.enter_ns,
        exit_ns: event.exit_ns,
        duration_ns: event.duration_ns,
    }
}

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

pub(crate) fn immediate_cause_tags(
    event: &SchedulerEvent,
    _stats: &metrics::TaskStats,
    fault_deltas: (u64, u64),
) -> Vec<String> {
    let mut tags = Vec::new();

    if event.observed_runnable_depth >= 4 {
        tags.push("runqueue_contention".to_string());
    }

    if event.target_pending_wakeups > 2 {
        tags.push("monitored_wakeup_backlog".to_string());
    }

    if fault_deltas.0 > 0 {
        tags.push("major_page_fault".to_string());
    } else if fault_deltas.1 > 0 {
        tags.push("minor_page_fault".to_string());
    }

    if event.wakeup_target_cpu != event.cpu {
        tags.push("migration_or_cpu_mismatch".to_string());
    }

    tags
}

pub(crate) fn primary_from_tags(tags: &[String]) -> Option<String> {
    let priority = [
        "major_page_fault",
        "runqueue_contention",
        "cpu_frequency",
        "irq_interference",
        "gpu_frame_pressure",
        "block_io",
        "migration_or_cpu_mismatch",
        "monitored_wakeup_backlog",
    ];

    priority
        .iter()
        .find(|candidate| tags.iter().any(|tag| tag == **candidate))
        .map(|cause| cause.to_string())
}

#[cfg(test)]
mod tests {
    use stutter_common::EVENT_RUNNABLE_LATENCY;

    use super::*;

    #[test]
    fn test_unaligned_event_decoding() {
        let event = SchedulerEvent {
            kind: EVENT_RUNNABLE_LATENCY,
            pid: 123,
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
        assert_eq!(decoded.pid, 123);
    }

    #[test]
    fn test_immediate_cause_tags() {
        let mut event = SchedulerEvent {
            kind: EVENT_RUNNABLE_LATENCY,
            pid: 123,
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
}
