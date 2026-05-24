//! MonitorSession orchestration and run loop.

use super::*;
use crate::session::ticks::foreground::foreground_event_for_final_metadata;
#[path = "monitor_session/run_loop.rs"]
mod run_loop;

pub struct MonitorSession {
    pub config: Arc<MonitorConfig>,
    pub runtime: MonitorRuntime,

    pub cpu_to_pkg: BTreeMap<u32, String>,

    pub hwmon_reader: Option<Arc<std::sync::Mutex<hwmon::HwmonReader>>>,
    pub(crate) gpu_engine_reader:
        Option<Arc<std::sync::Mutex<crate::gpu_engine::MultiGpuHwmonReader>>>,
    pub focus_resolver: Option<FocusResolver>,
    pub current_focus: Option<ResolvedFocus>,
    pub focus_switch_count: u64,
    pub foreground_resolver: Option<crate::foreground::ForegroundResolver>,
    pub current_foreground: Option<crate::foreground::ForegroundWindowSnapshot>,
    pub foreground_switch_count: u64,
    pub wayland_presentation_reader: Option<WaylandPresentationLogReader>,
    pub dmabuf_reader: Option<DmaBufLogReader>,

    pub started: Instant,
    pub had_tree_roots: bool,
    pub interval_label: &'static str,
    community_rules: crate::community_rules::CommunityRulesStatus,
}

impl MonitorSession {
    pub async fn new(
        config: MonitorConfig,
        shared_hwmon: Option<Arc<std::sync::Mutex<hwmon::HwmonReader>>>,
        event_tx: Option<tokio::sync::mpsc::Sender<MonitorEvent>>,
    ) -> anyhow::Result<Self> {
        let target_policy = TargetPolicy::from_monitor_config(&config)?;
        let target_plan = SessionTargetPlan::resolve(&config).await?;
        let probe_plan = SessionProbePlan::load(&config, &target_policy)?;
        let mut recorder = RecordingRuntime::begin(&config, &probe_plan)?;
        let exporter_runtime = ExporterRuntime::begin(&config, &mut recorder).await?;
        let alert_runtime = AlertRuntime::begin(&config);
        let sampler_runtime = SamplerRuntime::begin(&config);

        let metadata = crate::metadata::collect_system_metadata();
        let cpu_to_pkg: BTreeMap<u32, String> = metadata
            .cpu_topology
            .iter()
            .map(|c| (c.cpu, c.physical_package_id.clone().unwrap_or_default()))
            .collect();

        let hwmon_runtime = HwmonRuntime::begin(&config, shared_hwmon);
        let started = Instant::now();
        let wayland_presentation_reader = if config.probes.wayland_presentation {
            config
                .wayland_presentation
                .log_path
                .as_ref()
                .map(|path| WaylandPresentationLogReader::open_tail(path))
                .transpose()?
        } else {
            None
        };
        let dmabuf_reader = if config.probes.dmabuf_tracking {
            config
                .dmabuf
                .log_path
                .as_ref()
                .map(|path| DmaBufLogReader::open_tail(path))
                .transpose()?
        } else {
            None
        };

        let ui = UiRuntimeStage::begin(&config)?;
        let event_runtime_config = crate::events::EventRuntimeConfig::from_monitor_config(&config);

        let interval_label = if config.timing.epoch_period_ms.is_some() {
            "epoch"
        } else {
            "summary"
        };

        let probes = ProbeRuntime::new(
            probe_plan.loaded,
            probe_plan.block_io_correlation_basis,
            probe_plan.block_io_correlation_confidence,
            probe_plan.native_cgroup_filter,
            sampler_runtime.cpu_perf_sampler,
            sampler_runtime.runtime_slice_sampler,
        );

        let targeting = TargetController::from_policy_parts(
            target_policy,
            target_plan.watch_config,
            target_plan.tree_pids,
            target_plan.watch_state,
            target_plan.tree_root_starttimes,
        );

        let outputs = OutputRuntime::from_parts(
            recorder,
            exporter_runtime.prometheus_state,
            exporter_runtime.prometheus_task,
            exporter_runtime.otel_exporter,
            alert_runtime.sender,
            event_runtime_config.output,
        );
        let runtime = MonitorRuntime::from_config_parts(
            probes,
            outputs,
            ui,
            targeting,
            MonitorEventBus::new(event_tx),
            event_runtime_config,
        );
        Ok(Self {
            config: Arc::new(config),
            runtime,
            cpu_to_pkg,
            hwmon_reader: hwmon_runtime.reader,
            gpu_engine_reader: hwmon_runtime.engine_reader,
            community_rules: target_plan.community_rules,
            focus_resolver: target_plan.focus_resolver,
            current_focus: target_plan.current_focus,
            focus_switch_count: 0,
            foreground_resolver: target_plan.foreground_resolver,
            current_foreground: target_plan.current_foreground,
            foreground_switch_count: 0,
            wayland_presentation_reader,
            dmabuf_reader,
            started,
            had_tree_roots: target_plan.had_tree_roots,
            interval_label,
        })
    }

    pub fn emit(
        &mut self,
        event: MonitorEvent,
    ) -> impl std::future::Future<Output = ()> + Send + '_ {
        self.runtime.bus.emit(event)
    }

    pub(crate) async fn refresh_tasks_and_emit_snapshot(&mut self) -> anyhow::Result<()> {
        let elapsed_ms = self.started.elapsed().as_millis() as u64;
        let previous_active_targets: BTreeSet<u32> = self
            .runtime
            .targeting
            .tasks
            .active_targets
            .keys()
            .copied()
            .collect();

        self.refresh_tasks().await?;

        let removed_targets = previous_active_targets
            .into_iter()
            .filter(|tid| {
                !self
                    .runtime
                    .targeting
                    .tasks
                    .active_targets
                    .contains_key(tid)
            })
            .collect::<Vec<_>>();

        self.dispatch_monitor_event(MonitorEvent::TargetSnapshot {
            elapsed_ms,
            active_targets: self.runtime.targeting.tasks.active_targets.clone(),
            removed_targets,
        })
        .await?;

        Ok(())
    }

    pub(crate) async fn dispatch_monitor_event(
        &mut self,
        event: MonitorEvent,
    ) -> anyhow::Result<()> {
        let output = self.runtime.event_runtime_config.output;
        let outputs = &mut self.runtime.outputs;
        let mut sinks = MonitorOutputSinks::new(
            output,
            &mut outputs.recorder,
            outputs.alert_sender.as_ref(),
            &mut outputs.sink_registry,
        );

        if let Err(err) = sinks.dispatch(&event) {
            warn!("monitor_event_sink_failed err={err}");
        }

        self.emit(event).await;
        Ok(())
    }

    fn handle_ctrl_c_stop(&self) -> String {
        "ctrl_c".to_owned()
    }

    fn handle_max_duration_stop(&self, reason: Option<String>) -> String {
        reason.unwrap_or_else(|| "max_duration".to_owned())
    }

    fn handle_remote_stop(&self) -> String {
        "remote_stop".to_owned()
    }

    fn handle_epoch_tick(&self) -> Option<String> {
        self.config
            .timing
            .epoch_period_ms
            .is_some()
            .then(|| "epoch_ended".to_owned())
    }

    async fn handle_target_tick(
        &mut self,
        context: TargetTickContext,
    ) -> anyhow::Result<Option<String>> {
        match context.event {
            TargetTickEvent::Tree => self.handle_tree_tick().await,
            TargetTickEvent::Watch => {
                self.handle_watch_tick().await?;
                Ok(None)
            }
        }
    }

    async fn handle_focus_context_tick(
        &mut self,
        _context: FocusTickContext,
    ) -> anyhow::Result<()> {
        self.handle_focus_tick().await
    }

    async fn handle_foreground_context_tick(
        &mut self,
        _context: ForegroundTickContext,
    ) -> anyhow::Result<()> {
        self.handle_foreground_tick().await
    }

    async fn handle_summary_context_tick(
        &mut self,
        _context: SummaryTickContext,
    ) -> anyhow::Result<()> {
        self.handle_summary_tick().await
    }

    async fn handle_probe_drain(&mut self, _context: ProbeDrainContext) -> anyhow::Result<()> {
        self.drain_bpf_events().await
    }

    async fn handle_frame_tick(&mut self, context: FrameTickContext) -> anyhow::Result<()> {
        let frame = context.frame;
        self.dispatch_monitor_event(MonitorEvent::Frame {
            event: Box::new(frame.clone()),
        })
        .await?;
        self.runtime.telemetry.push_frame(frame);
        Ok(())
    }

    async fn handle_wayland_presentation_tick(
        &mut self,
        context: WaylandPresentationTickContext,
    ) -> anyhow::Result<()> {
        self.dispatch_monitor_event(MonitorEvent::WaylandPresentationEvent {
            event: Box::new(context.event),
        })
        .await
    }

    async fn handle_dmabuf_tick(&mut self, context: DmaBufTickContext) -> anyhow::Result<()> {
        self.dispatch_monitor_event(MonitorEvent::DmaBufEvent {
            event: Box::new(context.event),
        })
        .await
    }

    fn normalize_wayland_presentation_event(
        &self,
        mut event: recorder::WaylandPresentationEventRecord,
    ) -> recorder::WaylandPresentationEventRecord {
        let timestamp_ns = event.presented_ns.or(event.commit_ns);
        if let (Some(start_ns), Some(timestamp_ns)) = (
            self.runtime
                .outputs
                .recorder
                .run
                .as_ref()
                .and_then(|run| run.monotonic_start_ns),
            timestamp_ns,
        ) && let Some(delta_ns) = timestamp_ns.checked_sub(start_ns)
        {
            event.elapsed_ms = delta_ns / 1_000_000;
        } else if event.elapsed_ms == 0 {
            event.elapsed_ms = self.started.elapsed().as_millis() as u64;
        }
        event
    }

    fn normalize_dmabuf_event(
        &self,
        mut event: recorder::DmaBufEventRecord,
    ) -> recorder::DmaBufEventRecord {
        if event.elapsed_ms == 0 {
            event.elapsed_ms = self.started.elapsed().as_millis() as u64;
        }
        event
    }

    async fn handle_wayland_presentation_log_tick(&mut self) -> anyhow::Result<()> {
        let events = if let Some(reader) = &mut self.wayland_presentation_reader {
            reader.read_new_events()?
        } else {
            Vec::new()
        };

        for event in events {
            let event = self.normalize_wayland_presentation_event(event);
            self.handle_wayland_presentation_tick(WaylandPresentationTickContext { event })
                .await?;
        }

        Ok(())
    }

    async fn handle_dmabuf_log_tick(&mut self) -> anyhow::Result<()> {
        let events = if let Some(reader) = &mut self.dmabuf_reader {
            reader.read_new_events()?
        } else {
            Vec::new()
        };

        for event in events {
            let event = self.normalize_dmabuf_event(event);
            self.handle_dmabuf_tick(DmaBufTickContext { event }).await?;
        }

        Ok(())
    }

    async fn handle_telemetry_tick(&mut self, context: TelemetryTickContext) -> anyhow::Result<()> {
        match context.event {
            TelemetryTickEvent::MangoHudAlignment {
                raw_ms,
                monotonic_ns,
            } => {
                if let Some(run) = self.runtime.outputs.recorder.run.as_mut() {
                    run.mangohud_first_frame_raw_elapsed_ms = Some(raw_ms);
                    run.mangohud_first_frame_monotonic_ns = Some(monotonic_ns);
                    info!(
                        "mangohud_alignment_observed raw_ms={} monotonic_ns={}",
                        raw_ms, monotonic_ns
                    );
                }
                Ok(())
            }
            TelemetryTickEvent::Scx => self.handle_scx_tick().await,
            TelemetryTickEvent::Hwmon => self.handle_hwmon_tick().await,
        }
    }

    fn handle_ui_tick(&mut self, context: UiTickContext) -> Option<String> {
        self.handle_tui_event(context.event)
    }

    pub fn handle_tui_event(&mut self, event: Event) -> Option<String> {
        if let Event::Key(key) = event {
            match key.code {
                KeyCode::Char('q') | KeyCode::Char('Q') => return Some("quit".to_owned()),
                KeyCode::Char('p') | KeyCode::Char('P') => {
                    self.runtime.ui.tui_state.paused = !self.runtime.ui.tui_state.paused;
                }
                KeyCode::Char('s') | KeyCode::Char('S') => {
                    self.runtime.ui.tui_state.sort_field =
                        self.runtime.ui.tui_state.sort_field.next();
                }
                KeyCode::Char('f') | KeyCode::Char('F') => {
                    self.runtime.ui.tui_state.next_filter_class();
                }
                _ => {}
            }
        }
        None
    }

    pub async fn handle_summary_tick(&mut self) -> anyhow::Result<()> {
        if !self.runtime.ui.tui_state.paused {
            let elapsed_ms = self.started.elapsed().as_millis() as u64;
            if let Some(sampler) = self.runtime.probes.cpu_perf_sampler.as_mut() {
                let deltas = sampler.sample_interval();
                for (tid, delta) in deltas {
                    if let Some(stats) = self.runtime.targeting.tasks.stats_by_task.get_mut(&tid) {
                        stats.record_cpu_perf(&delta);
                    }
                }
            }

            let drop_counters_snapshot = self.runtime.probes.loaded.snapshot_drop_counters();
            let psi_snapshot = self.runtime.probes.psi_reader.read_with_delta().ok();
            let records = collect_interval_summaries_labeled(
                self.interval_label,
                &mut self.runtime.targeting.tasks.stats_by_task,
                elapsed_ms,
                &drop_counters_snapshot,
                self.runtime.probes.loaded.prev_faults_map.as_ref(),
                psi_snapshot.as_ref(),
                &mut self.runtime.targeting.tasks.prev_faults_snapshot,
            );

            self.dispatch_monitor_event(MonitorEvent::Interval {
                elapsed_ms,
                records: records.clone(),
                drop_counters: drop_counters_snapshot.clone(),
            })
            .await?;

            if let Some(sampler) = self.runtime.probes.runtime_slice_sampler.as_mut() {
                let tasks = self
                    .runtime
                    .targeting
                    .tasks
                    .active_targets
                    .values()
                    .cloned()
                    .collect::<Vec<_>>();
                let batch = sampler.collect(
                    &tasks,
                    elapsed_ms,
                    self.config.timing.summary_period_ms,
                    self.config.runtime_slices.max_tasks,
                );
                self.runtime
                    .outputs
                    .recorder
                    .counters
                    .runtime_slice_read_errors = self
                    .runtime
                    .outputs
                    .recorder
                    .counters
                    .runtime_slice_read_errors
                    .saturating_add(batch.read_errors);
                self.runtime
                    .outputs
                    .recorder
                    .counters
                    .runtime_slice_skipped_tasks = self
                    .runtime
                    .outputs
                    .recorder
                    .counters
                    .runtime_slice_skipped_tasks
                    .saturating_add(batch.skipped_tasks as u64);

                if self
                    .runtime
                    .outputs
                    .recorder
                    .streams
                    .contains(ArtifactKind::RuntimeSlices)
                {
                    for record in &batch.records {
                        crate::artifacts::push_artifact_event(
                            &mut self.runtime.outputs.recorder,
                            ArtifactKind::RuntimeSlices,
                            record,
                            "runtime_slices",
                            |c| {
                                c.runtime_slice_count += 1;
                            },
                        );
                    }
                } else {
                    self.runtime.outputs.recorder.counters.runtime_slice_count = self
                        .runtime
                        .outputs
                        .recorder
                        .counters
                        .runtime_slice_count
                        .saturating_add(batch.records.len() as u64);
                }
            }

            if let Some(state) = self.runtime.outputs.prometheus_state.as_ref() {
                let max_p99 = records.iter().map(|r| r.p99_ns).max().unwrap_or(0);
                state.set_latest_p99_ns(max_p99);
                state.set_active_targets(self.runtime.targeting.tasks.active_targets.len() as u64);
                state.set_event_stream_write_errors(
                    self.runtime
                        .outputs
                        .recorder
                        .counters
                        .event_stream_write_errors,
                );
                state.set_ebpf_ringbuf_drops(drop_counters_snapshot.total());
            }
        }

        if let Some(term) = self.runtime.ui.terminal.as_mut() {
            let elapsed_ms = self.started.elapsed().as_millis() as u64;
            let drop_counters_snapshot = self.runtime.probes.loaded.snapshot_drop_counters();

            let snapshot = TuiRenderSnapshot {
                elapsed_ms,
                drop_counters: drop_counters_snapshot,
                tui_state: self.runtime.ui.tui_state.clone(),
                active_targets: self.runtime.targeting.tasks.active_targets.clone(),
                stats_by_task: self.runtime.targeting.tasks.stats_by_task.clone(),
                interval_records: self
                    .runtime
                    .outputs
                    .recorder
                    .buffers
                    .interval_records
                    .clone(),
                recent_diagnoses: self.runtime.telemetry.diagnoses.clone(),
                current_focus: self.current_focus.clone(),
                current_foreground: self.current_foreground.clone(),
                focus_switch_count: self.focus_switch_count,
                foreground_include_title: self.config.focus.foreground_include_title,
            };

            // TUI rendering errors and panics should be logged and dismissed,
            // not propagated, to avoid killing the monitor.
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                term.draw(move |f| {
                    crate::tui::render_tui(
                        f,
                        crate::tui::TuiRenderInput {
                            state: &snapshot.tui_state,
                            active_targets: &snapshot.active_targets,
                            stats_by_task: &snapshot.stats_by_task,
                            interval_records: &snapshot.interval_records,
                            recent_diagnoses: &snapshot.recent_diagnoses,
                            elapsed_ms: snapshot.elapsed_ms.into(),
                            drop_counters: &snapshot.drop_counters,
                            current_focus: snapshot.current_focus.as_ref(),
                            current_foreground: snapshot.current_foreground.as_ref(),
                            focus_switch_count: snapshot.focus_switch_count,
                            foreground_include_title: snapshot.foreground_include_title,
                        },
                    );
                })
            }));

            match res {
                Ok(Ok(_)) => {}
                Ok(Err(err)) => warn!("tui_render_failed err={err:#}"),
                Err(_) => {
                    warn!("tui_render_panic");
                }
            }
        }

        Ok(())
    }

    pub async fn handle_tree_tick(&mut self) -> anyhow::Result<Option<String>> {
        let mut should_exit = None;

        if let Some(root_pid) = self.runtime.targeting.watch_state.running_pid()
            && tree_root_is_stale(root_pid, &self.runtime.targeting.tree_root_starttimes)
        {
            self.runtime.targeting.remove_watch_root(root_pid);

            if !self.config.target.persistent {
                should_exit = Some("watched_process_exit".to_owned());
            } else {
                self.runtime.targeting.watch_state = WatchProcessState::Waiting;
                info!("watch_process_waiting_for_relaunch");
            }
        } else {
            let removed_roots = self.runtime.targeting.remove_stale_dynamic_tree_roots();

            for root in &removed_roots {
                info!("tree_root_removed pid={root}");
            }

            if !removed_roots.is_empty()
                && self.had_tree_roots
                && self.runtime.targeting.effective_tree_pids().is_empty()
                && !matches!(
                    self.runtime.targeting.watch_state,
                    WatchProcessState::Waiting
                )
            {
                should_exit = Some("tree_root_exit".to_owned());
            }
        }

        self.refresh_tasks_and_emit_snapshot().await?;

        // Belt-and-suspenders cleanup in case a refresh path exits before
        // emitting per-task removal diffs.
        self.runtime
            .targeting
            .tasks
            .prev_faults_snapshot
            .retain(|tid, _| {
                self.runtime
                    .targeting
                    .tasks
                    .active_targets
                    .contains_key(tid)
            });

        Ok(should_exit)
    }

    pub async fn handle_watch_tick(&mut self) -> anyhow::Result<()> {
        let Some(pattern) = self.runtime.targeting.watch_config.pattern.clone() else {
            return Ok(());
        };

        if !self.runtime.targeting.watch_state.should_poll() {
            return Ok(());
        }

        if let Some(pid) = find_process_by_pattern_at_with_cache(
            Path::new("/proc"),
            &pattern,
            &mut self.runtime.targeting.process_cache,
        ) {
            self.runtime.targeting.add_watch_root(pid);
            self.runtime.targeting.watch_state = WatchProcessState::Running(pid);
            info!("watch_process_relaunched pattern={} pid={}", pattern, pid);

            self.refresh_tasks_and_emit_snapshot().await?;
        }

        Ok(())
    }

    pub async fn handle_scx_tick(&mut self) -> anyhow::Result<()> {
        if let Some(event) = self
            .runtime
            .probes
            .scx_tracker
            .sample(self.started.elapsed().as_millis() as u64)
        {
            self.dispatch_monitor_event(MonitorEvent::ScxEvent {
                event: Box::new(event),
            })
            .await?;
        }

        Ok(())
    }

    pub async fn handle_hwmon_tick(&mut self) -> anyhow::Result<()> {
        if let Some(reader_arc) = &self.hwmon_reader {
            let elapsed = self.started.elapsed().as_millis() as u64;
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

            if let Some(sample) = sample_opt {
                self.runtime.telemetry.push_gpu(sample.clone());
                self.dispatch_monitor_event(MonitorEvent::GpuSample {
                    sample: Box::new(sample),
                })
                .await?;
            }
        }
        if let Some(reader_arc) = &self.gpu_engine_reader {
            let elapsed = self.started.elapsed().as_millis() as u64;
            let reader_arc_clone = reader_arc.clone();

            let samples = task::spawn_blocking(move || {
                if let Ok(mut reader) = reader_arc_clone.lock() {
                    reader.sample(elapsed)
                } else {
                    Vec::new()
                }
            })
            .await
            .map_err(|err| anyhow::anyhow!("gpu engine worker failed: {err}"))?;

            for sample in samples {
                self.dispatch_monitor_event(MonitorEvent::GpuEngineSample {
                    sample: Box::new(sample),
                })
                .await?;
            }
        }
        Ok(())
    }

    async fn handle_live_spike(&mut self, spike: SpikeEvent) -> anyhow::Result<()> {
        let elapsed_ms = self.started.elapsed().as_millis() as u64;
        self.runtime.telemetry.push_spike(spike);
        // Prune old telemetry (history window controlled by LiveTelemetry::max_age_ms)
        self.runtime.telemetry.prune(elapsed_ms.into());

        let cluster_window_ms = self.config.diagnosis.live_cluster_window_ms;
        let cluster_window_ns = cluster_window_ms * 1_000_000;

        let recent_spikes: Vec<_> = self
            .runtime
            .telemetry
            .spikes
            .iter()
            .filter(|s| elapsed_ms.saturating_sub(s.elapsed_ms.unwrap_or(0)) <= cluster_window_ms)
            .cloned()
            .collect();

        if recent_spikes.is_empty() {
            return Ok(());
        }

        let mut points = Vec::new();
        for s in &recent_spikes {
            points.push(crate::spike::SpikePoint {
                task: s.task.as_u32(),
                class: s.class,
                process_pid: s.process_pid.map(|pid| pid.as_u32()),
                comm: s.comm.clone(),
                cpu: s.cpu,
                wakeup_target_cpu: s.wakeup_target_cpu,
                latency_ns: s.latency_ns,
                wakeup_ns: s.wakeup_ns,
                switch_ns: s.switch_ns,
                target_pending_wakeups: s.target_pending_wakeups,
                observed_runnable_depth: s.observed_runnable_depth,
                switch_prev_pid: s.switch_prev_pid.as_u32(),
                switch_prev_state: s.switch_prev_state,
                elapsed_ms: s.elapsed_ms,
                scx_ops: s.scx_ops.clone(),
                scx_state: s.scx_state.clone(),
                cause_tags: s.cause_tags.clone(),
                primary_cause: s.primary_cause.clone(),
                ..Default::default()
            });
        }

        let distinct_tasks = points.iter().map(|p| p.task).collect::<HashSet<_>>().len();
        let min_switch_ns = points.iter().map(|p| p.switch_ns).min().unwrap_or(0);
        let max_switch_ns = points.iter().map(|p| p.switch_ns).max().unwrap_or(0);
        let max_latency_ns = points.iter().map(|p| p.latency_ns).max().unwrap_or(0);

        let mut cluster = crate::spike::SpikeCluster {
            points,
            distinct_tasks,
            min_switch_ns,
            max_switch_ns,
            max_latency_ns,
            ..Default::default()
        };

        // Build a RunArtifacts-like snapshot from recent telemetry and let
        // `diagnose_cluster` perform time filtering itself.
        let artifacts = crate::session_io::RunArtifacts {
            irq_events: self.runtime.telemetry.irq_events.iter().cloned().collect(),
            gpu_samples: self.runtime.telemetry.gpu_samples.iter().cloned().collect(),
            block_io_events: self.runtime.telemetry.io_events.iter().cloned().collect(),
            intervals: self
                .runtime
                .outputs
                .recorder
                .buffers
                .interval_records
                .clone(),
            ..Default::default()
        };

        let diagnosis = diagnose_cluster(&cluster, &artifacts, cluster_window_ns);
        let anchor = crate::diagnosis::select_anchor(&cluster);
        cluster.anchor_task = Some(anchor.task);
        cluster.anchor_class = Some(anchor.class);
        cluster.anchor_comm = Some(anchor.comm.clone());
        cluster.anchor_kind = Some(anchor.kind);

        if diagnosis.cause != crate::diagnosis::StutterCause::Unknown {
            let entry = LiveDiagnosisEntry {
                elapsed_ms,
                cause: diagnosis.cause,
                confidence: diagnosis.confidence,
                anchor_class: anchor.class,
                anchor_comm: anchor.comm.clone(),
                evidence: diagnosis.evidence.clone(),
            };

            self.runtime.telemetry.diagnoses.push_back(entry.clone());
            self.dispatch_monitor_event(MonitorEvent::LiveDiagnosis {
                entry: Box::new(entry),
            })
            .await?;

            log::info!(
                "live_diagnosis cause={:?} confidence={:?} evidence={:?}",
                diagnosis.cause,
                diagnosis.confidence,
                diagnosis.evidence
            );
        }

        Ok(())
    }

    pub async fn refresh_tasks(&mut self) -> anyhow::Result<()> {
        let targeting = &mut self.runtime.targeting;
        let policy = &targeting.policy;
        let dynamic_tree_pids = &targeting.dynamic_tree_pids;
        let process_cache = &mut targeting.process_cache;
        let tasks = &mut targeting.tasks;
        let target_snapshot_input = TargetController::target_snapshot_input_from_parts(
            policy,
            dynamic_tree_pids,
            process_cache,
            self.community_rules.as_db(),
        );

        let budget_report = tasks
            .refresh(crate::tasks::RefreshInput {
                target_snapshot_input,
                max_tasks: policy.max_tasks,
                tree_events: &mut self.runtime.outputs.recorder.buffers.tree_events,
                target_pid_map: &mut self.runtime.probes.loaded.target_pid_map,
                prev_faults_map: self.runtime.probes.loaded.prev_faults_map.as_mut(),
                elapsed_ms: self.started.elapsed().as_millis() as u64,
                recording_started: self
                    .runtime
                    .outputs
                    .recorder
                    .run
                    .as_ref()
                    .map(|run| run.started_instant),
            })
            .await?;

        if budget_report.scan_timed_out {
            self.runtime
                .outputs
                .recorder
                .counters
                .process_scan_budget_exceeded_count += 1;
        }

        self.runtime
            .outputs
            .recorder
            .counters
            .thread_scan_limited_count += budget_report.processes_thread_limited as u64;

        if let Some(sampler) = self.runtime.probes.cpu_perf_sampler.as_mut() {
            sampler.sync_targets(
                &self.runtime.targeting.tasks.active_targets,
                &self.runtime.targeting.tasks.stats_by_task,
            );
        }

        Ok(())
    }

    pub fn finalize(mut self, stop_reason: String) -> anyhow::Result<String> {
        if let Some(term) = self.runtime.ui.terminal.as_mut() {
            let _ = crate::tui::restore_terminal(term);
        }

        let drop_counters = self.runtime.probes.loaded.snapshot_drop_counters();
        log_drop_counters(&drop_counters);

        if let Some(dropped) = self
            .runtime
            .outputs
            .recorder
            .exporters
            .otel_spans_dropped
            .as_ref()
        {
            let count = dropped.load(std::sync::atomic::Ordering::Relaxed);
            let stdout_is_machine_stream =
                self.config.outputs.json_stream || self.config.csv_streams_to_stdout();
            if count > 0 && !stdout_is_machine_stream {
                println!("OpenTelemetry export: dropped {count} spans due to channel pressure");
            }
        }
        let stdout_is_machine_stream =
            self.config.outputs.json_stream || self.config.csv_streams_to_stdout();
        if self.config.timing.epoch_period_ms.is_none() && !stdout_is_machine_stream {
            print_session_summaries(&mut self.runtime.targeting.tasks.stats_by_task);
        }

        if let Some(writer) = self.runtime.outputs.recorder.csv_writer.as_mut() {
            writer.finish()?;
            if let Some(CsvStreamTarget::File(path)) = &self.config.streams.csv
                && !self.config.outputs.json_stream
            {
                println!("wrote interval CSV: {}", path.display());
            }
        }

        if self.runtime.outputs.recorder.run.is_some() {
            let frame_events = crate::session::mangohud_frames::read_and_stream_non_live_events(
                self.config.as_ref(),
                &mut self.runtime.outputs.recorder,
            );

            self.runtime.outputs.recorder.streams.finish_all()?;

            recorder::finalize_recording(FinalizeRecordingInput {
                recorder: &self.runtime.outputs.recorder,
                config: &self.config,
                tree_pids: self.runtime.targeting.effective_tree_pids(),
                stop_reason: &stop_reason,
                tasks: &self.runtime.targeting.tasks,
                frame_events: &frame_events,
                block_io_correlation_basis: self.runtime.probes.block_io_correlation_basis.clone(),
                block_io_correlation_confidence: self
                    .runtime
                    .probes
                    .block_io_correlation_confidence
                    .clone(),
                native_cgroup_filter: self.runtime.probes.native_cgroup_filter.clone(),
                probe_activation_warnings: self.runtime.probes.recorded_activation_warnings(),
                focus_mode: if self.config.focus.auto_focus {
                    Some("auto".to_owned())
                } else if self.config.has_explicit_target() {
                    Some("explicit".to_owned())
                } else {
                    Some("legacy-auto-detect".to_owned())
                },
                final_focus_kind: self
                    .current_focus
                    .as_ref()
                    .map(|focus| format!("{:?}", focus.group.kind)),
                focus_switch_count: self.focus_switch_count,
                current_focus: self.current_focus.clone(),
                final_foreground_event: foreground_event_for_final_metadata(
                    self.current_foreground.as_ref(),
                    self.runtime.outputs.recorder.last_foreground_event.as_ref(),
                    self.config.focus.foreground_include_title,
                ),
                drop_counters,
                cpu_perf_status: self
                    .runtime
                    .probes
                    .cpu_perf_sampler
                    .as_ref()
                    .map(|sampler| recorder::CpuPerfStatus {
                        sample_count: sampler.total_samples(),
                        active_counter_tasks: sampler.active_counter_tasks() as u64,
                        skipped_counter_tasks: sampler.skipped_counter_tasks() as u64,
                        open_errors: sampler.total_open_errors(),
                        read_errors: sampler.total_read_errors(),
                        last_error: sampler.last_error().map(str::to_owned),
                    }),
            })?;

            recorder::print_recording_warnings(&self.runtime.outputs.recorder);
        }

        info!("exiting stop_reason={stop_reason}");
        Ok(stop_reason)
    }
}

fn display_driver_from_source(source: &str) -> Option<String> {
    match source {
        "amdgpu" | "amdgpu_tracepoint" => Some("amdgpu".to_owned()),
        "i915" | "i915_tracepoint" => Some("i915".to_owned()),
        _ => None,
    }
}
