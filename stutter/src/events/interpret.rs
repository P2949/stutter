use std::time::Instant;

use stutter_common::{EVENT_RUNNABLE_LATENCY, SchedulerEvent};

use crate::{
    cli::Config,
    events::{AlertPayload, immediate_cause_tags, primary_from_tags},
    metrics::{self},
    recorder,
    session_events::MonitorEvent,
    tasks::{TaskTracker, should_replace_unknown_comm},
};

#[derive(Debug, Default)]
pub struct SchedulerSampleUpdate {
    pub events: Vec<MonitorEvent>,
    pub spike_event: Option<recorder::SpikeEvent>,
}

#[allow(clippy::too_many_arguments)]
pub fn interpret_scheduler_event(
    event: &SchedulerEvent,
    config: &Config,
    started: Instant,
    tasks: &mut TaskTracker,
    monotonic_start_ns: Option<u64>,
    scx_ops: Option<&str>,
    scx_state: Option<&str>,
    scx_enable_seq: Option<&str>,
) -> SchedulerSampleUpdate {
    debug_assert_eq!(event.kind, EVENT_RUNNABLE_LATENCY);

    let comm = metrics::comm_to_string(&event.comm);
    let elapsed_ms = started.elapsed().as_millis() as u64;

    let task_info = tasks
        .active_targets
        .get(&event.tid)
        .or_else(|| tasks.known_targets.get(&event.tid))
        .cloned();

    let active = tasks.active_targets.contains_key(&event.tid);

    let waker_comm = tasks
        .stats_by_task
        .get(&event.waker_tid)
        .map(|stats| stats.comm.clone())
        .or_else(|| {
            tasks
                .active_targets
                .get(&event.waker_tid)
                .map(|target| target.comm.clone())
        })
        .or_else(|| tasks.cache.comm_for_tid(event.waker_tid))
        .unwrap_or_default();

    let stats = tasks
        .stats_by_task
        .entry(event.tid)
        .or_insert_with(|| metrics::TaskStats::new(event.tid, comm.clone(), elapsed_ms));

    if should_replace_unknown_comm(&stats.comm, &comm) {
        stats.comm = comm.clone();
    }

    if let Some(task_info) = task_info.as_ref() {
        stats.apply_task_info(task_info);
        stats.active = active;
    } else if config.cgroupv2.is_some() {
        stats.active = true;
    }

    let precomputed_fault_deltas = (
        event.maj_flt.saturating_sub(stats.last_spike_major_faults),
        event.min_flt.saturating_sub(stats.last_spike_minor_faults),
    );

    let spike_cause_tags_and_primary = if event.latency_ns >= config.spike_threshold_ns {
        let cause_tags = immediate_cause_tags(event, stats, precomputed_fault_deltas);
        let primary_cause = primary_from_tags(&cause_tags);
        Some((cause_tags, primary_cause))
    } else {
        None
    };

    let record_diagnostics =
        spike_cause_tags_and_primary
            .as_ref()
            .map(
                |(cause_tags, primary_cause)| metrics::SpikeRecordDiagnostics {
                    scx_ops: scx_ops.map(str::to_owned),
                    scx_state: scx_state.map(str::to_owned),
                    scx_enable_seq: scx_enable_seq.map(str::to_owned),
                    cause_tags: cause_tags.clone(),
                    primary_cause: primary_cause.clone(),
                },
            );

    let fault_deltas = stats.record(
        event,
        config.spike_threshold_ns,
        elapsed_ms,
        record_diagnostics,
    );

    let mut events = Vec::with_capacity(3);

    let is_spike = event.latency_ns >= config.spike_threshold_ns;
    let label = if is_spike {
        if config.verbose { "sample" } else { "spike" }
    } else {
        "sample"
    };

    events.push(MonitorEvent::SchedulerSample {
        event: Box::new(*event),
        comm: comm.clone(),
        label,
    });

    let mut spike_event = None;

    if is_spike {
        let (cause_tags, primary_cause) = spike_cause_tags_and_primary
            .expect("spike cause tags must be computed for spike events");

        let event_record = recorder::SpikeEvent::from_task_stats(
            monotonic_start_ns,
            stats,
            event,
            fault_deltas,
            recorder::SpikeDiagnosticContext {
                scx_ops: scx_ops.map(str::to_owned),
                scx_state: scx_state.map(str::to_owned),
                scx_enable_seq: scx_enable_seq.map(str::to_owned),
                cause_tags,
                primary_cause,
                waker_tid: event.waker_tid,
                waker_comm,
            },
        );

        if let Some(threshold) = config.alert_threshold_ns
            && event.latency_ns >= threshold
        {
            events.push(MonitorEvent::Alert {
                payload: Box::new(AlertPayload::from_task_stats(
                    stats,
                    event,
                    elapsed_ms,
                    scx_ops,
                    scx_state,
                    scx_enable_seq,
                )),
            });
        }

        events.push(MonitorEvent::Spike {
            event: Box::new(event_record.clone()),
        });
        spike_event = Some(event_record);
    }

    SchedulerSampleUpdate {
        events,
        spike_event,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config {
            monitor_config_layer: None,
            preset: None,
            target_pids: Vec::new(),
            tree_pids: Vec::new(),
            summary_period_ms: 1000,
            epoch_period_ms: None,
            spike_threshold_ns: 1_000_000,
            alert_threshold_ns: None,
            alert_webhook_url: None,
            verbose: false,
            task_filters: crate::process_tree::TaskFilters {
                include_comm: Vec::new(),
                exclude_comm: Vec::new(),
            },
            keep_missing_pid: false,
            watch_process: None,
            persistent: false,
            watch_poll_ms: 1000,
            watch_timeout: None,
            max_tasks: 1024,
            csv_stream: None,
            irq_latency: false,
            irqs: Vec::new(),
            hwmon: false,
            hwmon_root: None,
            hwmon_drm_card: None,
            hwmon_render_node: None,
            mangohud_log: None,
            tui: false,
            retain_intervals: None,
            recording: None,
            max_duration: None,
            cpu_freq: true,
            cgroupv2: None,
            native_cgroup_filter: false,
            follow_exec: true,
            exclude_tree_pids: Vec::new(),
            faults: false,
            cpu_perf: false,
            cpu_perf_kernel: false,
            cpu_perf_max_tasks: 128,
            cpu_perf_cache_refs: false,
            block_io: false,
            stat_wait: false,
            runtime_slices: false,
            runtime_slices_max_tasks: 256,
            json_stream: false,
            mangohud_log_live: false,
            metrics_port: None,
            ringbuf_size_kb: None,
            wakeup_map_factor: None,
            otlp_endpoint: None,
            otel_service_name: "stutter".to_owned(),
            auto_focus: false,
            focus_source: crate::cli::FocusSource::Heuristic,
            foreground_window: false,
            foreground_source: crate::cli::ForegroundSourceArg::Auto,
            foreground_poll_ms: 1000,
            foreground_max_stale_ms: 2500,
            foreground_include_title: false,
            auto_focus_poll_ms: 1000,
            auto_focus_min_confidence: 0.60,
            auto_focus_switch_cooldown_ms: 5000,
            auto_focus_switch_margin: 0.20,
            auto_focus_required_polls: 2,
            auto_focus_max_roots: 4,
            remote: None,
        }
    }

    fn scheduler_event(latency_ns: u64) -> SchedulerEvent {
        SchedulerEvent {
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
            wakeup_ns: 10,
            switch_ns: 20,
            latency_ns,
            comm: [0; 16],
            switch_prev_pid: 0,
            switch_prev_state: 0,
        }
    }

    #[test]
    fn non_spike_interpretation_emits_scheduler_sample_only() {
        let config = config();
        let mut tasks = TaskTracker::default();

        let update = interpret_scheduler_event(
            &scheduler_event(10),
            &config,
            Instant::now(),
            &mut tasks,
            None,
            None,
            None,
            None,
        );

        assert_eq!(update.events.len(), 1);
        assert!(matches!(
            update.events[0],
            MonitorEvent::SchedulerSample { .. }
        ));
        assert!(update.spike_event.is_none());
    }

    #[test]
    fn spike_interpretation_emits_scheduler_sample_and_spike() {
        let config = config();
        let mut tasks = TaskTracker::default();

        let update = interpret_scheduler_event(
            &scheduler_event(2_000_000),
            &config,
            Instant::now(),
            &mut tasks,
            None,
            None,
            None,
            None,
        );

        assert_eq!(update.events.len(), 2);
        assert!(matches!(
            update.events[0],
            MonitorEvent::SchedulerSample { .. }
        ));
        assert!(matches!(update.events[1], MonitorEvent::Spike { .. }));
        assert!(update.spike_event.is_some());
    }

    #[test]
    fn alert_threshold_emits_alert_event() {
        let mut config = config();
        config.alert_threshold_ns = Some(1_500_000);
        let mut tasks = TaskTracker::default();

        let update = interpret_scheduler_event(
            &scheduler_event(2_000_000),
            &config,
            Instant::now(),
            &mut tasks,
            None,
            None,
            None,
            None,
        );

        assert!(
            update
                .events
                .iter()
                .any(|event| matches!(event, MonitorEvent::Alert { .. }))
        );
    }
}
