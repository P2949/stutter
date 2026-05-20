//! MonitorSession orchestration and run loop.

use super::*;

pub struct MonitorSession {
    pub config: Arc<MonitorConfig>,
    pub runtime: MonitorRuntime,

    pub cpu_to_pkg: BTreeMap<u32, String>,

    pub hwmon_reader: Option<Arc<std::sync::Mutex<hwmon::HwmonReader>>>,
    pub focus_resolver: Option<FocusResolver>,
    pub current_focus: Option<ResolvedFocus>,
    pub focus_switch_count: u64,
    pub foreground_resolver: Option<crate::foreground::ForegroundResolver>,
    pub current_foreground: Option<crate::foreground::ForegroundWindowSnapshot>,
    pub foreground_switch_count: u64,
    pub wayland_presentation_reader: Option<WaylandPresentationLogReader>,

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
            community_rules: target_plan.community_rules,
            focus_resolver: target_plan.focus_resolver,
            current_focus: target_plan.current_focus,
            focus_switch_count: 0,
            foreground_resolver: target_plan.foreground_resolver,
            current_foreground: target_plan.current_foreground,
            foreground_switch_count: 0,
            wayland_presentation_reader,
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

    pub async fn run(
        &mut self,
        mut stop_rx: Option<tokio::sync::oneshot::Receiver<()>>,
    ) -> anyhow::Result<String> {
        let mut summary_tick =
            interval(Duration::from_millis(self.config.timing.summary_period_ms));
        summary_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

        let epoch_tick_duration = self
            .config
            .timing
            .epoch_period_ms
            .map(Duration::from_millis)
            .unwrap_or(Duration::from_secs(3600 * 24 * 365));
        let mut epoch_tick = interval(epoch_tick_duration);
        epoch_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

        let mut tree_tick = if needs_tree_tick_from_parts(
            self.had_tree_roots,
            self.runtime.targeting.watch_config.is_active(),
            self.config.target.cgroupv2.is_some(),
        ) {
            let mut tick = interval(Duration::from_millis(2_000));
            tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
            Some(tick)
        } else {
            None
        };

        let mut focus_tick = if self.focus_resolver.is_some() {
            let mut tick = interval(Duration::from_millis(self.config.focus.auto_focus_poll_ms));
            tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
            Some(tick)
        } else {
            None
        };

        let mut foreground_tick = if self.foreground_resolver.is_some() {
            let mut tick = interval(Duration::from_millis(self.config.focus.foreground_poll_ms));
            tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
            Some(tick)
        } else {
            None
        };

        let mut watch_tick = interval(Duration::from_millis(
            self.runtime.targeting.watch_config.poll_ms,
        ));
        watch_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

        let mut scx_tick = interval(Duration::from_millis(1_000));
        scx_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

        let mut hwmon_tick = interval(Duration::from_millis(1_000));
        hwmon_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

        let mut wayland_presentation_tick = if self.wayland_presentation_reader.is_some() {
            let mut tick = interval(Duration::from_millis(100));
            tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
            Some(tick)
        } else {
            None
        };

        let mut tui_event_reader = if self.config.ui.tui {
            Some(crossterm::event::EventStream::new())
        } else {
            None
        };

        let max_duration = self.config.timing.max_duration;
        let max_duration_future = async move {
            if let Some(duration) = max_duration {
                tokio::time::sleep(duration).await;
                Some("max_duration_reached".to_owned())
            } else {
                futures_util::future::pending().await
            }
        };
        tokio::pin!(max_duration_future);

        self.refresh_tasks_and_emit_snapshot().await?;

        let (mangohud_tx, mut mangohud_rx) = tokio::sync::oneshot::channel::<(u64, u64)>();
        let (frame_tx, mut frame_rx) = tokio::sync::mpsc::channel(1024);

        if let Some(run) = self.runtime.outputs.recorder.run.as_ref()
            && let Some(path) = self.config.mangohud.log.clone()
            && let Some(offset) = run.mangohud_start_offset
        {
            let path_clone = path.clone();
            tokio::spawn(async move {
                if let Ok(res) = mangohud::poll_alignment(&path_clone, offset).await {
                    let _ = mangohud_tx.send(res);
                }
            });

            if self.config.mangohud.log_live {
                tokio::spawn(async move {
                    if let Err(err) = mangohud::tail_frames(path, offset, frame_tx).await {
                        log::warn!("mangohud_tail_failed err={err:#}");
                    }
                });
            }
        }

        loop {
            tokio::select! {
                res = &mut mangohud_rx => {
                    if let Ok((raw_ms, monotonic_ns)) = res {
                        self.handle_telemetry_tick(TelemetryTickContext {
                            event: TelemetryTickEvent::MangoHudAlignment {
                                raw_ms,
                                monotonic_ns,
                            },
                        })
                        .await?;
                    }
                }
                Some(frame) = frame_rx.recv() => {
                    self.handle_frame_tick(FrameTickContext { frame }).await?;
                }
                _ = tokio::signal::ctrl_c() => {
                    return Ok(self.handle_ctrl_c_stop());
                }
                reason = &mut max_duration_future => {
                    return Ok(self.handle_max_duration_stop(reason));
                }
                _ = async {
                    if let Some(rx) = &mut stop_rx {
                        let _ = rx.await;
                        Some(())
                    } else {
                        futures_util::future::pending().await
                    }
                } => {
                    return Ok(self.handle_remote_stop());
                }

                _ = summary_tick.tick() => {
                    self.handle_summary_context_tick(SummaryTickContext).await?;
                }

                _ = epoch_tick.tick() => {
                    if let Some(reason) = self.handle_epoch_tick() {
                        return Ok(reason);
                    }
                }

                _ = optional_tick(tree_tick.as_mut()) => {
                    if let Some(reason) = self.handle_target_tick(TargetTickContext {
                        event: TargetTickEvent::Tree,
                    }).await? {
                        return Ok(reason);
                    }
                }

                _ = optional_tick(focus_tick.as_mut()) => {
                    self.handle_focus_context_tick(FocusTickContext).await?;
                }

                _ = optional_tick(foreground_tick.as_mut()) => {
                    self.handle_foreground_context_tick(ForegroundTickContext).await?;
                }

                _ = watch_tick.tick() => {
                    self.handle_target_tick(TargetTickContext {
                        event: TargetTickEvent::Watch,
                    }).await?;
                }

                _ = scx_tick.tick() => {
                    self.handle_telemetry_tick(TelemetryTickContext {
                        event: TelemetryTickEvent::Scx,
                    }).await?;
                }

                _ = hwmon_tick.tick() => {
                    self.handle_telemetry_tick(TelemetryTickContext {
                        event: TelemetryTickEvent::Hwmon,
                    }).await?;
                }

                _ = optional_tick(wayland_presentation_tick.as_mut()) => {
                    self.handle_wayland_presentation_log_tick().await?;
                }

                maybe_event = async {
                    if let Some(reader) = &mut tui_event_reader {
                        futures_util::StreamExt::next(reader).await
                    } else {
                        futures_util::future::pending().await
                    }
                } => {
                    if let Some(Ok(event)) = maybe_event
                        && let Some(reason) = self.handle_ui_tick(UiTickContext { event })
                    {
                        return Ok(reason);
                    }
                }

                res = self.handle_probe_drain(ProbeDrainContext) => {
                    res?;
                }
            }
        }
    }

    async fn drain_bpf_events(&mut self) -> anyhow::Result<()> {
        let mut guard = self.runtime.probes.loaded.events.readable_mut().await?;
        let recording_monotonic_start_ns = self
            .runtime
            .outputs
            .recorder
            .run
            .as_ref()
            .and_then(|r| r.monotonic_start_ns);

        let mut pending_spikes = Vec::new();
        let mut pending_irqs = Vec::new();
        let mut pending_ios = Vec::new();
        let mut pending_monitor_events = Vec::new();

        let current_scx = scx_snapshot(&self.runtime.probes.scx_tracker);

        while let Some(item) = guard.get_inner_mut().next() {
            if item.len() < std::mem::size_of::<u32>() {
                log::warn!("short_bpf_event len={}", item.len());
                continue;
            }

            let kind = unsafe { (item.as_ptr() as *const u32).read_unaligned() };
            match kind {
                stutter_common::EVENT_RUNNABLE_LATENCY => {
                    if let Some(event) = crate::events::decode::read_event_unaligned::<
                        stutter_common::SchedulerEvent,
                    >(&item)
                    {
                        let update = crate::events::handle_event_with_runtime_config(
                            &event,
                            crate::events::EventHandlingContext {
                                config: &self.runtime.event_runtime_config,
                                started: self.started,
                                tasks: &mut self.runtime.targeting.tasks,
                                monotonic_start_ns: recording_monotonic_start_ns,
                                diagnostics: crate::events::interpret::SchedulerEventDiagnostics {
                                    scx_ops: current_scx.ops.as_deref(),
                                    scx_state: current_scx.state.as_deref(),
                                    scx_enable_seq: current_scx.enable_seq.as_deref(),
                                },
                            },
                        );
                        let crate::events::interpret::SchedulerSampleUpdate {
                            events,
                            spike_event,
                        } = update;
                        if let Some(spike) = spike_event {
                            pending_spikes.push(spike);
                        }
                        pending_monitor_events.extend(events);
                    } else {
                        log::warn!("short_scheduler_event len={}", item.len());
                    }
                }
                stutter_common::EVENT_IRQ_LATENCY => {
                    if let Some(event) = crate::events::decode::read_event_unaligned::<
                        stutter_common::IrqEvent,
                    >(&item)
                    {
                        let record =
                            crate::events::irq_event_record(recording_monotonic_start_ns, &event);
                        pending_monitor_events.push(crate::events::handle_irq_record(&record));
                        pending_irqs.push(record);
                    } else {
                        log::warn!("short_irq_event len={}", item.len());
                    }
                }
                stutter_common::EVENT_MIGRATION => {
                    if let Some(event) = crate::events::decode::read_event_unaligned::<
                        stutter_common::MigrationEvent,
                    >(&item)
                    {
                        pending_monitor_events.push(crate::events::handle_migration_event(
                            &event,
                            &mut self.runtime.targeting.tasks,
                            &self.cpu_to_pkg,
                            self.started,
                        ));
                    } else {
                        log::warn!("short_migration_event len={}", item.len());
                    }
                }
                stutter_common::EVENT_CPU_FREQ => {
                    if let Some(event) = crate::events::decode::read_event_unaligned::<
                        stutter_common::CpuFreqEvent,
                    >(&item)
                    {
                        pending_monitor_events
                            .push(crate::events::handle_cpu_freq_event(&event, self.started));
                    } else {
                        log::warn!("short_cpu_freq_event len={}", item.len());
                    }
                }
                stutter_common::EVENT_STAT_WAIT => {
                    if let Some(event) = crate::events::decode::read_event_unaligned::<
                        stutter_common::StatWaitEvent,
                    >(&item)
                    {
                        if let Some(stats) = self
                            .runtime
                            .targeting
                            .tasks
                            .stats_by_task
                            .get_mut(&event.tid)
                        {
                            stats.stat_wait_sum_ns += event.delay_ns as u128;
                            stats.stat_wait_count += 1;
                        }
                    } else {
                        log::warn!("short_stat_wait_event len={}", item.len());
                    }
                }
                stutter_common::EVENT_BLOCK_IO => {
                    if let Some(event) = crate::events::decode::read_event_unaligned::<
                        stutter_common::BlockIoEvent,
                    >(&item)
                    {
                        let record = crate::events::block_io_event_record(
                            &event,
                            self.runtime
                                .probes
                                .loaded
                                .block_io_correlation_basis
                                .as_str(),
                            self.started,
                        );

                        pending_monitor_events.push(crate::events::handle_block_io_record(&record));
                        pending_ios.push(record);
                    } else {
                        log::warn!("short_block_io_event len={}", item.len());
                    }
                }
                stutter_common::EVENT_EXEC => {
                    if self.config.safety.follow_exec {
                        let elapsed_ms = self.started.elapsed().as_millis() as u64;
                        if let Some(event) = crate::events::handle_exec_event(
                            &item,
                            &mut self.runtime.targeting.tasks,
                            elapsed_ms,
                        ) {
                            pending_monitor_events.push(event);
                        }
                    }
                }
                stutter_common::EVENT_KMS_FLIP => {
                    if let Some(event) = crate::events::decode::read_event_unaligned::<
                        stutter_common::KmsFlipEvent,
                    >(&item)
                    {
                        let elapsed_ms = elapsed_ms_from_event_timestamp(
                            recording_monotonic_start_ns,
                            event.timestamp_ns,
                        )
                        .unwrap_or_else(|| self.started.elapsed().as_millis() as u64);
                        let flags = kms_flip_flag_names(event.flags);
                        pending_monitor_events.push(
                            crate::session_events::MonitorEvent::KmsFlipEvent {
                                event: Box::new(crate::recorder::KmsFlipEventRecord {
                                    elapsed_ms,
                                    timestamp_ns: event.timestamp_ns,
                                    source: kms_flip_provider_name(event.provider).to_owned(),
                                    card: (event.card_minor != 0)
                                        .then(|| format!("card{}", event.card_minor)),
                                    driver: None,
                                    crtc_id: (event.flags & stutter_common::KMS_FLIP_HAS_CRTC != 0)
                                        .then_some(event.crtc_id),
                                    connector: None,
                                    event_kind: kms_flip_event_kind_name(event.event_kind)
                                        .to_owned(),
                                    sequence: (event.flags & stutter_common::KMS_FLIP_HAS_SEQUENCE
                                        != 0)
                                        .then_some(event.sequence),
                                    request_ns: (event.flags
                                        & stutter_common::KMS_FLIP_HAS_REQUEST_NS
                                        != 0)
                                        .then_some(event.request_ns),
                                    done_ns: (event.flags & stutter_common::KMS_FLIP_HAS_DONE_NS
                                        != 0)
                                        .then_some(event.done_ns),
                                    duration_ns: (event.flags
                                        & stutter_common::KMS_FLIP_HAS_DURATION_NS
                                        != 0)
                                        .then_some(event.duration_ns),
                                    flags,
                                    confidence: if event.flags
                                        & stutter_common::KMS_FLIP_HAS_DURATION_NS
                                        != 0
                                    {
                                        "medium".to_owned()
                                    } else {
                                        "low".to_owned()
                                    },
                                }),
                            },
                        );
                    } else {
                        log::warn!("short_kms_flip_event len={}", item.len());
                    }
                }
                stutter_common::EVENT_DRM_FENCE => {
                    if let Some(event) = crate::events::decode::read_event_unaligned::<
                        stutter_common::DrmFenceEvent,
                    >(&item)
                    {
                        let elapsed_ms = elapsed_ms_from_event_timestamp(
                            recording_monotonic_start_ns,
                            event.timestamp_ns,
                        )
                        .unwrap_or_else(|| self.started.elapsed().as_millis() as u64);
                        let has_context_seqno = event.flags
                            & (stutter_common::DRM_FENCE_HAS_CONTEXT
                                | stutter_common::DRM_FENCE_HAS_SEQNO)
                            == (stutter_common::DRM_FENCE_HAS_CONTEXT
                                | stutter_common::DRM_FENCE_HAS_SEQNO);
                        let has_timeline_seqno = event.flags
                            & (stutter_common::DRM_FENCE_HAS_TIMELINE
                                | stutter_common::DRM_FENCE_HAS_SEQNO)
                            == (stutter_common::DRM_FENCE_HAS_TIMELINE
                                | stutter_common::DRM_FENCE_HAS_SEQNO);
                        let has_duration =
                            event.flags & stutter_common::DRM_FENCE_HAS_DURATION != 0;
                        let has_importer =
                            event.flags & stutter_common::DRM_FENCE_IS_IMPORTER_SIDE != 0;
                        let has_exporter =
                            event.flags & stutter_common::DRM_FENCE_IS_EXPORTER_SIDE != 0;
                        let importer_driver = has_importer
                            .then(|| drm_fence_provider_name(event.provider).to_owned());
                        let exporter_driver = has_exporter.then(|| {
                            let mapped = drm_fence_provider_name(event.driver_id);
                            if mapped == "unknown" {
                                drm_fence_provider_name(event.provider).to_owned()
                            } else {
                                mapped.to_owned()
                            }
                        });
                        let correlation_basis = if has_context_seqno {
                            "context_seqno"
                        } else if has_timeline_seqno {
                            "timeline_seqno"
                        } else if has_importer && has_exporter {
                            "driver_time_overlap"
                        } else {
                            "unknown"
                        };
                        let confidence = if has_duration && has_context_seqno {
                            "high"
                        } else if has_duration
                            && (has_timeline_seqno || has_importer && has_exporter)
                        {
                            "medium"
                        } else {
                            "low"
                        };
                        pending_monitor_events.push(
                            crate::session_events::MonitorEvent::DrmFenceEvent {
                                event: Box::new(crate::recorder::DrmFenceEventRecord {
                                    elapsed_ms,
                                    timestamp_ns: event.timestamp_ns,
                                    source: drm_fence_provider_name(event.provider).to_owned(),
                                    event_kind: drm_fence_event_kind_name(event.event_kind)
                                        .to_owned(),
                                    driver: None,
                                    card: None,
                                    gpu_role: Some(drm_gpu_role_name(event.gpu_role).to_owned()),
                                    pid: (event.flags & stutter_common::DRM_FENCE_HAS_PID != 0)
                                        .then_some(event.pid),
                                    tid: (event.flags & stutter_common::DRM_FENCE_HAS_PID != 0)
                                        .then_some(event.tid),
                                    comm: None,
                                    context: (event.flags & stutter_common::DRM_FENCE_HAS_CONTEXT
                                        != 0)
                                        .then_some(event.context),
                                    seqno: (event.flags & stutter_common::DRM_FENCE_HAS_SEQNO != 0)
                                        .then_some(event.seqno),
                                    timeline_hash: (event.flags
                                        & stutter_common::DRM_FENCE_HAS_TIMELINE
                                        != 0)
                                        .then_some(event.timeline_hash),
                                    wait_start_ns: (event.wait_start_ns != 0)
                                        .then_some(event.wait_start_ns),
                                    wait_done_ns: (event.wait_done_ns != 0)
                                        .then_some(event.wait_done_ns),
                                    duration_ns: (event.flags
                                        & stutter_common::DRM_FENCE_HAS_DURATION
                                        != 0)
                                        .then_some(event.duration_ns),
                                    exporter_driver,
                                    importer_driver,
                                    correlation_basis: correlation_basis.to_owned(),
                                    confidence: confidence.to_owned(),
                                }),
                            },
                        );
                    } else {
                        log::warn!("short_drm_fence_event len={}", item.len());
                    }
                }
                other => log::warn!("unknown_bpf_event kind={other} len={}", item.len()),
            }
        }

        guard.clear_ready();
        drop(guard);

        for event in pending_monitor_events {
            self.dispatch_monitor_event(event).await?;
        }

        for irq in pending_irqs {
            self.runtime.telemetry.push_irq(irq);
        }
        for io in pending_ios {
            self.runtime.telemetry.push_io(io);
        }
        for spike in pending_spikes {
            self.handle_live_spike(spike).await?;
        }

        Ok(())
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
        Ok(())
    }

    async fn handle_live_spike(&mut self, spike: SpikeEvent) -> anyhow::Result<()> {
        let elapsed_ms = self.started.elapsed().as_millis() as u64;
        self.runtime.telemetry.push_spike(spike);
        // Prune old telemetry (history window controlled by LiveTelemetry::max_age_ms)
        self.runtime.telemetry.prune(elapsed_ms.into());

        // Form a cluster from spikes within cluster_window_ms
        let cluster_window_ms = LIVE_DIAGNOSIS_CLUSTER_WINDOW_MS;
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
                task: s.task,
                class: s.class,
                process_pid: s.process_pid,
                comm: s.comm.clone(),
                cpu: s.cpu,
                wakeup_target_cpu: s.wakeup_target_cpu,
                latency_ns: s.latency_ns,
                wakeup_ns: s.wakeup_ns,
                switch_ns: s.switch_ns,
                target_pending_wakeups: s.target_pending_wakeups,
                observed_runnable_depth: s.observed_runnable_depth,
                switch_prev_pid: s.switch_prev_pid,
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
            self.runtime.outputs.recorder.streams.finish_all()?;

            let frame_events = if !self.config.mangohud.log_live
                && let Some(path) = &self.config.mangohud.log
            {
                let (alignment_monotonic_ns, alignment_raw_elapsed_ms, mangohud_ignore_offset) =
                    if let Some(run) = self.runtime.outputs.recorder.run.as_ref() {
                        (
                            run.mangohud_first_frame_monotonic_ns,
                            run.mangohud_first_frame_raw_elapsed_ms,
                            run.mangohud_start_offset.unwrap_or(0),
                        )
                    } else {
                        (None, None, 0)
                    };

                match mangohud::read_frame_events(
                    path,
                    mangohud_ignore_offset,
                    alignment_monotonic_ns,
                    alignment_raw_elapsed_ms,
                    self.runtime
                        .outputs
                        .recorder
                        .run
                        .as_ref()
                        .and_then(|r| r.monotonic_start_ns),
                ) {
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
                final_foreground_event: self.runtime.outputs.recorder.last_foreground_event.clone(),
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
