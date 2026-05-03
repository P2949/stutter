use std::{collections::BTreeMap, time::Instant};

use log::{debug, info, warn};
use serde::Serialize;
use stutter_common::{
    BlockIoEvent, CpuFreqEvent, EVENT_BLOCK_IO, EVENT_CPU_FREQ, EVENT_EXEC, EVENT_IRQ_LATENCY,
    EVENT_MIGRATION, EVENT_RUNNABLE_LATENCY, EVENT_STAT_WAIT, ExecEvent, IrqEvent, MigrationEvent,
    SchedulerEvent, StatWaitEvent,
};

use crate::{
    cli::Config,
    metrics::{self, format_latency, print_event},
    process_tree::{self, TaskClass},
    recorder::{self, IrqEventRecord, JsonArrayWriter, LiveRecorder},
    tasks::{TaskTracker, should_replace_unknown_comm},
};

pub struct HandleEventInput<'a> {
    pub event: &'a SchedulerEvent,
    pub config: &'a Config,
    pub started: Instant,
    pub tasks: &'a mut TaskTracker,
    pub monotonic_start_ns: Option<u64>,
    pub recorder: &'a mut LiveRecorder,
    pub alert_sender: Option<&'a std::sync::mpsc::SyncSender<AlertPayload>>,
}

pub struct DrainBpfEventsInput<'a> {
    pub guard: tokio::io::unix::AsyncFdReadyMutGuard<'a, aya::maps::RingBuf<aya::maps::MapData>>,
    pub config: &'a Config,
    pub started: Instant,
    pub tasks: &'a mut TaskTracker,
    pub recorder: &'a mut LiveRecorder,
    pub cpu_to_pkg: &'a BTreeMap<u32, String>,
    pub block_io_correlation_basis: &'a str,
    pub alert_sender: Option<&'a std::sync::mpsc::SyncSender<AlertPayload>>,
}

pub fn drain_bpf_events(input: DrainBpfEventsInput<'_>) {
    let DrainBpfEventsInput {
        mut guard,
        config,
        started,
        tasks,
        recorder,
        cpu_to_pkg,
        block_io_correlation_basis,
        alert_sender,
    } = input;

    let recording_monotonic_start_ns = recorder.run.as_ref().and_then(|r| r.monotonic_start_ns);

    while let Some(item) = guard.get_inner_mut().next() {
        if item.len() < std::mem::size_of::<u32>() {
            warn!("short_bpf_event len={}", item.len());
            continue;
        }

        let kind = unsafe { (item.as_ptr() as *const u32).read_unaligned() };
        match kind {
            EVENT_RUNNABLE_LATENCY => {
                let Some(event) = cast_event::<SchedulerEvent>(&item) else {
                    warn!("short_scheduler_event len={}", item.len());
                    continue;
                };

                handle_event(HandleEventInput {
                    event,
                    config,
                    started,
                    tasks,
                    monotonic_start_ns: recording_monotonic_start_ns,
                    recorder,
                    alert_sender,
                });
            }
            EVENT_IRQ_LATENCY => {
                let Some(event) = cast_event::<IrqEvent>(&item) else {
                    warn!("short_irq_event len={}", item.len());
                    continue;
                };

                let record = irq_event_record(recording_monotonic_start_ns, event);
                if let Some(writer) = recorder.irq_event_writer.as_mut() {
                    push_json_stream_event(
                        writer,
                        &record,
                        &mut recorder.irq_event_count,
                        "irq_events",
                    );
                }
                log_irq_event(event);
            }
            EVENT_MIGRATION => {
                let Some(event) = cast_event::<MigrationEvent>(&item) else {
                    warn!("short_migration_event len={}", item.len());
                    continue;
                };

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
                    push_json_stream_event(
                        writer,
                        &record,
                        &mut recorder.migration_event_count,
                        "migration_events",
                    );
                }
            }
            EVENT_CPU_FREQ => {
                let Some(event) = cast_event::<CpuFreqEvent>(&item) else {
                    warn!("short_cpu_freq_event len={}", item.len());
                    continue;
                };

                let elapsed_ms = started.elapsed().as_millis();

                if let Some(writer) = recorder.cpu_freq_sample_writer.as_mut() {
                    let record = recorder::CpuFreqRecord {
                        elapsed_ms,
                        cpu: event.cpu,
                        freq_khz: event.state, // state field contains freq in kHz
                        timestamp_ns: event.timestamp_ns,
                    };
                    push_json_stream_event(
                        writer,
                        &record,
                        &mut recorder.cpu_freq_sample_count,
                        "cpu_freq_samples",
                    );
                }
            }
            EVENT_STAT_WAIT => {
                let Some(event) = cast_event::<StatWaitEvent>(&item) else {
                    warn!("short_stat_wait_event len={}", item.len());
                    continue;
                };

                if let Some(stats) = tasks.stats_by_task.get_mut(&event.tid) {
                    stats.stat_wait_sum_ns += event.delay_ns as u128;
                    stats.stat_wait_count += 1;
                }
            }
            EVENT_BLOCK_IO => {
                let Some(event) = cast_event::<BlockIoEvent>(&item) else {
                    warn!("short_block_io_event len={}", item.len());
                    continue;
                };

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
                    push_json_stream_event(
                        writer,
                        &record,
                        &mut recorder.block_io_event_count,
                        "io_events",
                    );
                }
            }
            EVENT_EXEC => {
                if !config.follow_exec {
                    continue;
                }
                if item.len() < std::mem::size_of::<ExecEvent>() {
                    warn!("short_exec_event len={}", item.len());
                    continue;
                }
                let event = unsafe { &*(item.as_ptr() as *const ExecEvent) };
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
            other => warn!("unknown_bpf_event kind={other} len={}", item.len()),
        }
    }

    guard.clear_ready();
}

pub fn push_json_stream_event<T: Serialize>(
    writer: &mut JsonArrayWriter,
    value: &T,
    count: &mut u64,
    stream_name: &str,
) {
    match writer.push(value) {
        Ok(()) => *count += 1,
        Err(err) => warn!("json_stream_write_failed stream={stream_name} err={err:#}"),
    }
}

pub fn handle_event(input: HandleEventInput<'_>) {
    let HandleEventInput {
        event,
        config,
        started,
        tasks,
        monotonic_start_ns,
        recorder,
        alert_sender,
    } = input;

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

    let fault_deltas = stats.record(event, config.spike_threshold_ns, elapsed_ms);

    let alert_payload = if config
        .alert_threshold_ns
        .is_some_and(|threshold| event.latency_ns >= threshold)
    {
        Some(AlertPayload::from_task_stats(stats, event, elapsed_ms))
    } else {
        None
    };

    if event.latency_ns >= config.spike_threshold_ns
        && let Some(spike_events) = recorder.spike_events.as_mut()
    {
        spike_events.push(recorder::SpikeEvent::from_task_stats(
            monotonic_start_ns,
            stats,
            event,
            fault_deltas,
        ));
    }

    if config.verbose {
        print_event(event, &comm, "sample");
    } else if event.latency_ns >= config.spike_threshold_ns {
        print_event(event, &comm, "spike");
    }

    if let Some(alert_payload) = alert_payload
        && let Some(sender) = alert_sender
        && let Err(err) = sender.try_send(alert_payload)
    {
        warn!("alert_send_failed err={err}");
    }
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
}

impl AlertPayload {
    pub fn from_task_stats(
        stats: &metrics::TaskStats,
        event: &SchedulerEvent,
        elapsed_ms: u128,
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
        }
    }
}

pub fn send_desktop_alert(payload: &AlertPayload) -> Result<(), String> {
    let status = std::process::Command::new("notify-send")
        .args([
            "--urgency=critical",
            payload.title.as_str(),
            payload.message.as_str(),
        ])
        .status()
        .map_err(|err| format!("failed to run notify-send: {err}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("notify-send exited with {status}"))
    }
}

pub fn send_webhook_alert(url: &str, payload: &AlertPayload) -> Result<(), String> {
    let body = serde_json::to_string(payload)
        .map_err(|err| format!("failed to serialize alert payload: {err}"))?;
    let status = std::process::Command::new("curl")
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
        .status()
        .map_err(|err| format!("failed to run curl: {err}"))?;

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

pub fn cast_event<T: aya::Pod>(data: &[u8]) -> Option<&T> {
    if data.len() < std::mem::size_of::<T>() {
        return None;
    }
    unsafe { Some(&*(data.as_ptr() as *const T)) }
}
