use std::{
    collections::BTreeMap,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use crossterm::event::{Event, KeyCode};
use log::{info, warn};
use tokio::{
    task,
    time::{MissedTickBehavior, interval},
};

use crate::{
    cli::Config,
    ebpf_loader,
    events::AlertPayload,
    hwmon, mangohud,
    metrics::{collect_interval_summaries_labeled, log_drop_counters, print_session_summaries},
    process_tree::{self, find_auto_target_pids},
    psi,
    recorder::{self, FinalizeRecordingInput, JsonArrayWriter, LiveRecorder, SpikeEventBuffer},
    scx,
    tasks::TaskTracker,
    watch::{
        WatchProcessState, add_watch_tree_pid, capture_tree_root_starttimes,
        find_process_by_pattern_at_with_cache, process_root_starttime, remove_stale_tree_roots,
        remove_watch_tree_pid, resolve_watch_process, tree_root_is_stale,
    },
};

pub struct MonitorSession {
    pub config: Arc<Config>,
    pub tree_pids: Vec<u32>,
    pub watch_state: WatchProcessState,
    pub tree_root_starttimes: BTreeMap<u32, Option<u64>>,
    pub recorder: LiveRecorder,
    pub tasks: TaskTracker,
    pub loaded: ebpf_loader::LoadedEbpf,

    pub cpu_to_pkg: BTreeMap<u32, String>,
    pub psi_reader: psi::PsiReader,
    pub scx_tracker: scx::ScxTracker,
    pub hwmon_reader: Option<Arc<std::sync::Mutex<hwmon::HwmonReader>>>,
    pub watch_process_cache: process_tree::ProcessCache,

    pub started: Instant,
    pub tui_state: crate::tui::TuiState,
    pub terminal: Option<ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>>,
    pub had_tree_roots: bool,
    pub interval_label: &'static str,
    pub block_io_correlation_basis: String,
    pub alert_sender: Option<tokio::sync::mpsc::Sender<AlertPayload>>,
}

impl MonitorSession {
    pub async fn new(
        mut config: Config,
        shared_hwmon: Option<Arc<std::sync::Mutex<hwmon::HwmonReader>>>,
    ) -> anyhow::Result<Self> {
        if config.target_pids.is_empty()
            && config.tree_pids.is_empty()
            && config.watch_process.is_none()
            && config.cgroupv2.is_none()
        {
            let auto_targets = find_auto_target_pids(Path::new("/proc"));
            if auto_targets.is_empty() {
                anyhow::bail!(
                    "no target specified and no game launcher (gamescope, pressure-vessel, etc.) detected. \
                     Please provide --pid <PID>, --tree-pid <PID>, --watch-process <COMM>, or --cgroupv2 <PATH>"
                );
            }

            let pids: Vec<_> = auto_targets.iter().map(|(p, _)| *p).collect();
            let class = auto_targets[0].1;
            info!("auto_detected_launcher class={class} pids={pids:?}");
            println!("auto-detected game launcher: {class} (PIDs {pids:?}). monitoring tree...");
            config.tree_pids = pids;
        }

        let mut tree_pids = config.tree_pids.clone();
        let watch_state = match resolve_watch_process(&config, &mut tree_pids).await? {
            Some(pid) => WatchProcessState::Running(pid),
            None => WatchProcessState::None,
        };

        let had_tree_roots = !tree_pids.is_empty();
        let tree_root_starttimes = capture_tree_root_starttimes(&tree_pids);

        let recording = recorder::prepare_recording(&config)?;
        let mut loaded = ebpf_loader::load_and_attach(&config)?;
        configure_target_irqs(&mut loaded, &config)?;
        let block_io_correlation_basis = loaded.block_io_correlation_basis.as_str().to_owned();

        let recorder = LiveRecorder {
            run: recording,
            interval_records: Vec::new(),
            tree_events: Vec::new(),
            spike_events: None, // Will set below
            irq_events: Vec::new(),
            gpu_samples: Vec::new(),

            interval_writer: None, // Will set below
            irq_event_writer: None,
            migration_event_writer: None,
            cpu_freq_sample_writer: None,
            gpu_sample_writer: None,
            block_io_event_writer: None,
            csv_writer: None,

            intervals_dropped: 0,
            scx_event_count: 0,
            irq_event_count: 0,
            migration_event_count: 0,
            cpu_freq_sample_count: 0,
            gpu_sample_count: 0,
            block_io_event_count: 0,
            interval_record_count: 0,
            spike_events_dropped_count: 0,
        };

        let mut recorder = recorder;
        recorder.spike_events = recorder.run.as_ref().map(|_| SpikeEventBuffer::default());

        if let Some(run) = recorder.run.as_ref() {
            if config.retain_intervals.is_none() {
                recorder.interval_writer =
                    Some(JsonArrayWriter::create(run.run_dir.join("interval.json"))?);
            }
            recorder.irq_event_writer = Some(JsonArrayWriter::create(
                run.run_dir.join("irq_events.json"),
            )?);
            recorder.migration_event_writer = Some(JsonArrayWriter::create(
                run.run_dir.join("migration_events.json"),
            )?);
            recorder.cpu_freq_sample_writer = Some(JsonArrayWriter::create(
                run.run_dir.join("cpu_freq_samples.json"),
            )?);
            recorder.gpu_sample_writer = Some(JsonArrayWriter::create(
                run.run_dir.join("gpu_samples.json"),
            )?);
            recorder.block_io_event_writer =
                Some(JsonArrayWriter::create(run.run_dir.join("io_events.json"))?);
        }

        if let Some(path) = &config.csv_path {
            recorder.csv_writer = Some(recorder::IntervalCsvWriter::create(path.clone())?);
        }

        let metadata = crate::metadata::collect_system_metadata();
        let cpu_to_pkg: BTreeMap<u32, String> = metadata
            .cpu_topology
            .iter()
            .map(|c| (c.cpu, c.physical_package_id.clone().unwrap_or_default()))
            .collect();

        let psi_reader = psi::PsiReader::new();
        let mut scx_tracker = scx::ScxTracker::default();

        let hwmon_reader = if let Some(shared) = shared_hwmon {
            Some(shared)
        } else if config.hwmon {
            hwmon::HwmonReader::discover_with_options(
                config.hwmon_root.as_deref(),
                config.hwmon_drm_card.as_deref(),
                config.hwmon_render_node.as_deref(),
            )
            .map(|r| Arc::new(std::sync::Mutex::new(r)))
        } else {
            None
        };

        if config.hwmon && hwmon_reader.is_none() {
            warn!("hwmon_requested_but_no_gpu_hwmon_found");
        }

        let started = Instant::now();
        scx_tracker.sample(0);

        let tui_state = crate::tui::TuiState::default();
        let terminal = if config.tui {
            Some(
                crate::tui::init_terminal()
                    .map_err(|e| anyhow::anyhow!("failed to init terminal: {e}"))?,
            )
        } else {
            None
        };

        let interval_label = if config.epoch_period_ms.is_some() {
            "epoch"
        } else {
            "summary"
        };

        let alert_sender = if config.alert_threshold_ns.is_some() {
            let (tx, mut rx) = tokio::sync::mpsc::channel(100);
            let webhook_url = config.alert_webhook_url.clone();
            tokio::spawn(async move {
                while let Some(payload) = rx.recv().await {
                    if let Err(err) = crate::events::send_desktop_alert(&payload).await {
                        warn!("desktop_alert_failed err={err}");
                    }
                    if let Some(url) = &webhook_url
                        && let Err(err) = crate::events::send_webhook_alert(url, &payload).await
                    {
                        warn!("webhook_alert_failed url={url} err={err}");
                    }
                }
            });
            Some(tx)
        } else {
            None
        };

        Ok(Self {
            config: Arc::new(config),
            tree_pids,
            watch_state,
            tree_root_starttimes,
            recorder,
            tasks: TaskTracker::default(),
            loaded,
            cpu_to_pkg,
            psi_reader,
            scx_tracker,
            hwmon_reader,
            watch_process_cache: process_tree::ProcessCache::default(),
            started,
            tui_state,
            terminal,
            had_tree_roots,
            interval_label,
            block_io_correlation_basis,
            alert_sender,
        })
    }

    pub async fn run(&mut self) -> anyhow::Result<String> {
        let mut summary_tick = interval(Duration::from_millis(self.config.summary_period_ms));
        summary_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

        let epoch_tick_duration = self
            .config
            .epoch_period_ms
            .map(Duration::from_millis)
            .unwrap_or(Duration::from_secs(3600 * 24 * 365));
        let mut epoch_tick = interval(epoch_tick_duration);
        epoch_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

        let mut tree_tick = interval(Duration::from_millis(2_000));
        tree_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

        let mut watch_tick = interval(Duration::from_millis(self.config.watch_poll_ms));
        watch_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

        let mut scx_tick = interval(Duration::from_millis(1_000));
        scx_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

        let mut hwmon_tick = interval(Duration::from_millis(1_000));
        hwmon_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

        let mut tui_event_reader = crossterm::event::EventStream::new();

        let max_duration = self.config.max_duration;
        let max_duration_future = async move {
            if let Some(duration) = max_duration {
                tokio::time::sleep(duration).await;
                Some("max_duration_reached".to_owned())
            } else {
                futures_util::future::pending().await
            }
        };
        tokio::pin!(max_duration_future);

        self.refresh_tasks().await?;

        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => return Ok("ctrl_c".to_owned()),
                reason = &mut max_duration_future => return Ok(reason.unwrap()),

                _ = summary_tick.tick() => {
                    self.handle_summary_tick()?;
                }

                _ = epoch_tick.tick() => {
                    if self.config.epoch_period_ms.is_some() {
                        return Ok("epoch_ended".to_owned());
                    }
                }

                _ = tree_tick.tick() => {
                    if let Some(reason) = self.handle_tree_tick().await? {
                        return Ok(reason);
                    }
                }

                _ = watch_tick.tick() => {
                    self.handle_watch_tick().await?;
                }

                _ = scx_tick.tick() => {
                    self.handle_scx_tick();
                }

                _ = hwmon_tick.tick() => {
                    self.handle_hwmon_tick().await?;
                }

                maybe_event = futures_util::StreamExt::next(&mut tui_event_reader) => {
                    if let Some(Ok(event)) = maybe_event
                        && let Some(reason) = self.handle_tui_event(event)
                    {
                        return Ok(reason);
                    }
                }

                ready = self.loaded.events.readable_mut() => {
                    let mut guard = ready?;
                    let recording_monotonic_start_ns = self.recorder.run.as_ref().and_then(|r| r.monotonic_start_ns);

                    while let Some(item) = guard.get_inner_mut().next() {
                        if item.len() < std::mem::size_of::<u32>() {
                            log::warn!("short_bpf_event len={}", item.len());
                            continue;
                        }

                        let kind = unsafe { (item.as_ptr() as *const u32).read_unaligned() };
                        match kind {
                            stutter_common::EVENT_RUNNABLE_LATENCY => {
                                if let Some(event) = crate::events::read_event_unaligned::<stutter_common::SchedulerEvent>(&item) {
                                    crate::events::handle_event(
                                        &event,
                                        &self.config,
                                        self.started,
                                        &mut self.tasks,
                                        recording_monotonic_start_ns,
                                        &mut self.recorder,
                                        self.alert_sender.as_ref(),
                                    );
                                } else {
                                    log::warn!("short_scheduler_event len={}", item.len());
                                }
                            }
                            stutter_common::EVENT_IRQ_LATENCY => {
                                if let Some(event) = crate::events::read_event_unaligned::<stutter_common::IrqEvent>(&item) {
                                    crate::events::handle_irq_event(&event, &mut self.recorder, recording_monotonic_start_ns);
                                } else {
                                    log::warn!("short_irq_event len={}", item.len());
                                }
                            }
                            stutter_common::EVENT_MIGRATION => {
                                if let Some(event) = crate::events::read_event_unaligned::<stutter_common::MigrationEvent>(&item) {
                                    crate::events::handle_migration_event(
                                        &event,
                                        &mut self.tasks,
                                        &mut self.recorder,
                                        &self.cpu_to_pkg,
                                        self.started,
                                    );
                                } else {
                                    log::warn!("short_migration_event len={}", item.len());
                                }
                            }
                            stutter_common::EVENT_CPU_FREQ => {
                                if let Some(event) = crate::events::read_event_unaligned::<stutter_common::CpuFreqEvent>(&item) {
                                    crate::events::handle_cpu_freq_event(&event, &mut self.recorder, self.started);
                                } else {
                                    log::warn!("short_cpu_freq_event len={}", item.len());
                                }
                            }
                            stutter_common::EVENT_STAT_WAIT => {
                                if let Some(event) = crate::events::read_event_unaligned::<stutter_common::StatWaitEvent>(&item) {
                                    if let Some(stats) = self.tasks.stats_by_task.get_mut(&event.tid) {
                                        stats.stat_wait_sum_ns += event.delay_ns as u128;
                                        stats.stat_wait_count += 1;
                                    }
                                } else {
                                    log::warn!("short_stat_wait_event len={}", item.len());
                                }
                            }
                            stutter_common::EVENT_BLOCK_IO => {
                                if let Some(event) = crate::events::read_event_unaligned::<stutter_common::BlockIoEvent>(&item) {
                                    crate::events::handle_block_io_event(
                                        &event,
                                        &mut self.recorder,
                                        self.loaded.block_io_correlation_basis.as_str(),
                                        self.started,
                                    );
                                } else {
                                    log::warn!("short_block_io_event len={}", item.len());
                                }
                            }
                            stutter_common::EVENT_EXEC => {
                                if self.config.follow_exec {
                                    crate::events::handle_exec_event(&item, &mut self.tasks);
                                }
                            }
                            other => log::warn!("unknown_bpf_event kind={other} len={}", item.len()),
                        }
                    }

                    guard.clear_ready();
                }
            }
        }
    }

    pub fn handle_tui_event(&mut self, event: Event) -> Option<String> {
        if let Event::Key(key) = event {
            match key.code {
                KeyCode::Char('q') | KeyCode::Char('Q') => return Some("quit".to_owned()),
                KeyCode::Char('p') | KeyCode::Char('P') => {
                    self.tui_state.paused = !self.tui_state.paused;
                }
                KeyCode::Char('s') | KeyCode::Char('S') => {
                    self.tui_state.sort_field = self.tui_state.sort_field.next();
                }
                KeyCode::Char('f') | KeyCode::Char('F') => {
                    self.tui_state.next_filter_class();
                }
                _ => {}
            }
        }
        None
    }

    pub fn handle_summary_tick(&mut self) -> anyhow::Result<()> {
        if !self.tui_state.paused {
            let elapsed_ms = self.started.elapsed().as_millis();
            let drop_counters_snapshot = self.loaded.snapshot_drop_counters();
            let psi_snapshot = self.psi_reader.read().ok();
            let records = collect_interval_summaries_labeled(
                self.interval_label,
                &mut self.tasks.stats_by_task,
                elapsed_ms,
                &drop_counters_snapshot,
                self.loaded.prev_faults_map.as_ref(),
                psi_snapshot.as_ref(),
                &mut self.tasks.prev_faults_snapshot,
            );
            self.recorder.interval_record_count += records.len() as u64;

            if let Some(writer) = self.recorder.interval_writer.as_mut() {
                for record in &records {
                    writer.push(record)?;
                }
            } else if self.config.retain_intervals.is_some() || self.config.tui {
                // For TUI sparklines we need interval_records
                for record in &records {
                    self.recorder.interval_records.push(record.clone());
                }

                let max_intervals = self.config.retain_intervals.unwrap_or(120);
                if self.recorder.interval_records.len() > max_intervals {
                    let drop_count = self.recorder.interval_records.len() - max_intervals;
                    self.recorder.interval_records.drain(0..drop_count);
                    if self.config.retain_intervals.is_some() {
                        self.recorder.intervals_dropped += drop_count as u64;
                    }
                }
            }

            if let Some(writer) = self.recorder.csv_writer.as_mut() {
                for record in &records {
                    writer.push(record)?;
                }
            }
        }

        if let Some(term) = self.terminal.as_mut() {
            let elapsed_ms = self.started.elapsed().as_millis();
            let drop_counters_snapshot = self.loaded.snapshot_drop_counters();

            let tui_state = &self.tui_state;
            let active_targets = &self.tasks.active_targets;
            let stats_by_task = &self.tasks.stats_by_task;
            let interval_records = &self.recorder.interval_records;

            // TUI rendering errors and panics should be logged and dismissed,
            // not propagated, to avoid killing the monitor.
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                let _ = term.draw(move |f| {
                    crate::tui::render_tui(
                        f,
                        tui_state,
                        active_targets,
                        stats_by_task,
                        interval_records,
                        elapsed_ms,
                        &drop_counters_snapshot,
                    );
                });
            }));

            match res {
                Ok(_) => {}
                Err(_) => {
                    warn!("tui_render_panic");
                }
            }
        }

        Ok(())
    }

    pub async fn handle_tree_tick(&mut self) -> anyhow::Result<Option<String>> {
        let mut should_exit = None;

        if let Some(root_pid) = self.watch_state.running_pid()
            && tree_root_is_stale(root_pid, &self.tree_root_starttimes)
        {
            remove_watch_tree_pid(&mut self.tree_pids, root_pid);
            self.tree_root_starttimes.remove(&root_pid);

            if !self.config.persistent {
                should_exit = Some("watched_process_exit".to_owned());
            } else {
                self.watch_state = WatchProcessState::Waiting;
                info!("watch_process_waiting_for_relaunch");
            }
        } else {
            let removed_roots = remove_stale_tree_roots(
                &mut self.tree_pids,
                &mut self.tree_root_starttimes,
                self.watch_state.running_pid(),
            );

            for root in &removed_roots {
                info!("tree_root_removed pid={root}");
            }

            if !removed_roots.is_empty()
                && self.had_tree_roots
                && self.tree_pids.is_empty()
                && !matches!(self.watch_state, WatchProcessState::Waiting)
            {
                should_exit = Some("tree_root_exit".to_owned());
            }
        }

        self.refresh_tasks().await?;

        // Belt-and-suspenders cleanup in case a refresh path exits before
        // emitting per-task removal diffs.
        self.tasks
            .prev_faults_snapshot
            .retain(|tid, _| self.tasks.active_targets.contains_key(tid));

        Ok(should_exit)
    }

    pub async fn handle_watch_tick(&mut self) -> anyhow::Result<()> {
        let Some(pattern) = self.config.watch_process.clone() else {
            return Ok(());
        };

        if let Some(pid) = find_process_by_pattern_at_with_cache(
            Path::new("/proc"),
            &pattern,
            &mut self.watch_process_cache,
        ) {
            add_watch_tree_pid(&mut self.tree_pids, pid);
            self.tree_root_starttimes
                .insert(pid, process_root_starttime(pid));
            self.watch_state = WatchProcessState::Running(pid);
            info!("watch_process_relaunched pattern={} pid={}", pattern, pid);

            self.refresh_tasks().await?;
        }

        Ok(())
    }

    pub fn handle_scx_tick(&mut self) {
        self.scx_tracker.sample(self.started.elapsed().as_millis());
    }

    pub async fn handle_hwmon_tick(&mut self) -> anyhow::Result<()> {
        if let Some(reader_arc) = &self.hwmon_reader {
            let elapsed = self.started.elapsed().as_millis();
            let reader_arc_clone = reader_arc.clone();

            let sample_opt = task::spawn_blocking(move || {
                if let Ok(mut reader) = reader_arc_clone.lock() {
                    Some(reader.sample(elapsed))
                } else {
                    None
                }
            })
            .await
            .map_err(|err| anyhow::anyhow!("hwmon worker failed: {err}"))?;

            if let Some(sample) = sample_opt
                && let Some(writer) = self.recorder.gpu_sample_writer.as_mut()
            {
                writer.push(&sample)?;
                self.recorder.gpu_sample_count += 1;
            }
        }
        Ok(())
    }

    pub async fn refresh_tasks(&mut self) -> anyhow::Result<()> {
        self.tasks
            .refresh(crate::tasks::RefreshInput {
                config: &self.config,
                tree_pids: &self.tree_pids,
                tree_events: &mut self.recorder.tree_events,
                target_pid_map: &mut self.loaded.target_pid_map,
                prev_faults_map: self.loaded.prev_faults_map.as_mut(),
                elapsed_ms: self.started.elapsed().as_millis(),
                recording_started: self.recorder.run.as_ref().map(|run| run.started_instant),
            })
            .await
    }

    pub fn finalize(mut self, stop_reason: String) -> anyhow::Result<()> {
        if let Some(term) = self.terminal.as_mut() {
            let _ = crate::tui::restore_terminal(term);
        }

        let drop_counters = self.loaded.snapshot_drop_counters();
        log_drop_counters(&drop_counters);
        if self.config.epoch_period_ms.is_none() {
            print_session_summaries(&mut self.tasks.stats_by_task);
        }

        if let Some(writer) = self.recorder.csv_writer.as_mut() {
            writer.finish()?;
            if let Some(path) = &self.config.csv_path {
                println!("wrote interval CSV: {}", path.display());
            }
        }

        if self.recorder.run.is_some() {
            if let Some(writer) = self.recorder.interval_writer.as_mut() {
                writer.finish()?;
            }
            if let Some(writer) = self.recorder.irq_event_writer.as_mut() {
                writer.finish()?;
            }
            if let Some(writer) = self.recorder.migration_event_writer.as_mut() {
                writer.finish()?;
            }
            if let Some(writer) = self.recorder.cpu_freq_sample_writer.as_mut() {
                writer.finish()?;
            }
            if let Some(writer) = self.recorder.gpu_sample_writer.as_mut() {
                writer.finish()?;
            }
            if let Some(writer) = self.recorder.block_io_event_writer.as_mut() {
                writer.finish()?;
            }

            let frame_events = if let Some(path) = &self.config.mangohud_log {
                match mangohud::read_frame_events(path, self.config.mangohud_ignore_offset) {
                    Ok(events) => events,
                    Err(err) => {
                        warn!(
                            "mangohud_log_read_failed path={} err={err:#}",
                            path.display()
                        );
                        Vec::new()
                    }
                }
            } else {
                Vec::new()
            };

            recorder::finalize_recording(FinalizeRecordingInput {
                recorder: &self.recorder,
                config: &self.config,
                tree_pids: &self.tree_pids,
                stop_reason: &stop_reason,
                tasks: &self.tasks,
                frame_events: &frame_events,
                block_io_correlation_basis: &self.block_io_correlation_basis,
                drop_counters,
            })?;
        }

        info!("exiting stop_reason={stop_reason}");
        Ok(())
    }
}

pub async fn run_monitor(
    config: Arc<Config>,
    shared_hwmon: Option<Arc<std::sync::Mutex<hwmon::HwmonReader>>>,
) -> anyhow::Result<()> {
    let mut session = MonitorSession::new((*config).clone(), shared_hwmon).await?;
    let stop_reason = session.run().await?;
    session.finalize(stop_reason)
}

pub fn configure_target_irqs(
    loaded: &mut ebpf_loader::LoadedEbpf,
    config: &Config,
) -> anyhow::Result<()> {
    if !config.irq_latency {
        return Ok(());
    }

    let Some(target_irq_map) = loaded.target_irq_map.as_mut() else {
        warn!("irq_latency_requested_but_map_missing");
        return Ok(());
    };

    if config.irqs.is_empty() {
        anyhow::bail!(
            "--irq-latency requires at least one explicit --irq <N>; inspect /proc/interrupts to find the IRQ number for your GPU or device"
        );
    }

    for irq in config.irqs.iter().copied() {
        target_irq_map.insert(irq, 1, 0)?;
        info!("irq_latency_target_added irq={irq}");
    }

    Ok(())
}
