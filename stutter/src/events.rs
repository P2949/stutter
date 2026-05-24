use std::{collections::BTreeMap, time::Instant};

use log::{debug, info, warn};
use stutter_common::{
    BlockIoEvent, CpuFreqEvent, ExecEvent, IrqEvent as RawIrqEvent, MigrationEvent, SchedulerEvent,
};

use crate::{
    metrics::{self, format_latency},
    process_tree,
    recorder::{self, IrqEventRecord},
    session::sinks::MonitorOutputConfig,
    session_events::MonitorEvent,
    tasks::TaskTracker,
};

pub mod decode;
pub mod domain;
pub mod interpret;

pub fn handle_irq_record(record: &IrqEventRecord) -> MonitorEvent {
    log_irq_record(record);
    MonitorEvent::IrqEvent {
        event: Box::new(record.clone()),
    }
}

pub fn handle_migration_event(
    event: &MigrationEvent,
    tasks: &mut TaskTracker,
    cpu_to_pkg: &BTreeMap<u32, String>,
    started: Instant,
) -> MonitorEvent {
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
        tid: event.tid.into(),
        from_cpu: event.from_cpu,
        to_cpu: event.to_cpu,
        timestamp_ns: event.timestamp_ns,
    };

    MonitorEvent::MigrationEvent {
        event: Box::new(record),
    }
}

pub fn handle_cpu_freq_event(event: &CpuFreqEvent, started: Instant) -> MonitorEvent {
    let elapsed_ms = started.elapsed().as_millis() as u64;

    let record = recorder::CpuFreqRecord {
        elapsed_ms,
        cpu: event.cpu,
        freq_khz: event.state,
        timestamp_ns: event.timestamp_ns,
    };

    MonitorEvent::CpuFreqSample {
        event: Box::new(record),
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
        tid: event.tid.into(),
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

pub fn handle_block_io_record(record: &recorder::BlockIoRecord) -> MonitorEvent {
    MonitorEvent::IoEvent {
        event: Box::new(record.clone()),
    }
}

pub fn handle_exec_event(
    item: &[u8],
    tasks: &mut TaskTracker,
    elapsed_ms: u64,
) -> Option<MonitorEvent> {
    let Some(event) = decode::read_event_unaligned::<ExecEvent>(item) else {
        warn!("short_exec_event len={}", item.len());
        return None;
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

    Some(MonitorEvent::Exec {
        elapsed_ms,
        pid: event.pid,
        tid: event.tid,
        comm,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventRuntimeConfig {
    pub spike: interpret::SpikeConfig,
    pub output: MonitorOutputConfig,
}

impl EventRuntimeConfig {
    pub fn from_monitor_config(config: &crate::config::model::MonitorConfig) -> Self {
        Self {
            spike: interpret::SpikeConfig {
                spike_threshold_ns: config.timing.spike_threshold_ns,
                alert_threshold_ns: config.alerts.threshold_ns,
                verbose: config.streams.verbose,
                cgroupv2_active: config.target.cgroupv2.is_some(),
            },
            output: MonitorOutputConfig {
                json_stream: config.outputs.json_stream,
                verbose: config.streams.verbose,
                retain_interval_limit: config
                    .recording
                    .retain_intervals
                    .or_else(|| config.ui.tui.then_some(120)),
                count_interval_retention_drops: config.recording.retain_intervals.is_some(),
            },
        }
    }
}

pub struct EventHandlingContext<'a> {
    pub config: &'a EventRuntimeConfig,
    pub started: Instant,
    pub tasks: &'a mut TaskTracker,
    pub monotonic_start_ns: Option<u64>,
    pub diagnostics: interpret::SchedulerEventDiagnostics<'a>,
}

pub fn handle_event_with_runtime_config(
    event: &SchedulerEvent,
    context: EventHandlingContext<'_>,
) -> interpret::SchedulerSampleUpdate {
    interpret::interpret_scheduler_event(interpret::SchedulerEventInput {
        event,
        config: &context.config.spike,
        started: context.started,
        tasks: context.tasks,
        monotonic_start_ns: context.monotonic_start_ns,
        diagnostics: context.diagnostics,
    })
}

pub fn irq_event_record(monotonic_start_ns: Option<u64>, event: &RawIrqEvent) -> IrqEventRecord {
    let event = domain::IrqEvent::from_raw(*event);
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

pub fn log_irq_event(event: &RawIrqEvent) {
    let event = domain::IrqEvent::from_raw(*event);
    debug!(
        "irq_event cpu={} irq={} latency={}",
        event.cpu,
        event.irq,
        format_latency(event.duration_ns)
    );
}

#[cfg(test)]
mod tests {
    use stutter_common::{EVENT_BLOCK_IO, EVENT_EXEC, EVENT_IRQ_LATENCY, ExecEvent, IrqEvent};

    use super::*;

    #[test]
    fn irq_event_record_preserves_event_fields() {
        let event = IrqEvent {
            kind: EVENT_IRQ_LATENCY,
            irq: 44,
            cpu: 3,
            _pad0: 0,
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
            _pad0: 0,
            enter_ns: 1_000_000,
            exit_ns: 1_250_000,
            duration_ns: 250_000,
        };

        let record = irq_event_record(None, &event);

        assert_eq!(record.elapsed_ms, None);
    }

    #[test]
    fn handle_exec_event_updates_task_identity_and_returns_event() {
        let mut comm = [0u8; 16];
        comm[..8].copy_from_slice(b"game.exe");
        let raw = ExecEvent {
            kind: EVENT_EXEC,
            pid: 44,
            tid: 99,
            comm,
        };
        let bytes = unsafe {
            std::slice::from_raw_parts(
                (&raw as *const ExecEvent).cast::<u8>(),
                std::mem::size_of::<ExecEvent>(),
            )
        };

        let mut tasks = TaskTracker::default();
        tasks
            .stats_by_task
            .insert(99, metrics::TaskStats::new(99, "?".to_owned(), 0));

        let event = handle_exec_event(bytes, &mut tasks, 1234).unwrap();

        assert_eq!(tasks.stats_by_task.get(&99).unwrap().comm, "game.exe");

        match event {
            MonitorEvent::Exec {
                elapsed_ms,
                pid,
                tid,
                comm,
            } => {
                assert_eq!(elapsed_ms, 1234);
                assert_eq!(pid, 44);
                assert_eq!(tid, 99);
                assert_eq!(comm, "game.exe");
            }
            other => panic!("expected exec event, got {other:?}"),
        }
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
