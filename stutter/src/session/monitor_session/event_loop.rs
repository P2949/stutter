use super::{display::display_driver_from_source, *};

impl MonitorSession {
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
            let mut tick = interval(Duration::from_millis(
                crate::session::targeting::tree_tick_interval_ms(&self.config),
            ));
            tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
            Some(tick)
        } else {
            None
        };

        let mut focus_tick = if self.handles.target_refresh.focus_resolver.is_some() {
            let mut tick = interval(Duration::from_millis(self.config.focus.auto_focus_poll_ms));
            tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
            Some(tick)
        } else {
            None
        };

        let mut foreground_tick = if self.handles.target_refresh.foreground_resolver.is_some() {
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

        let mut dmabuf_tick = if self.dmabuf_reader.is_some() {
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

        let (mangohud_tx, mangohud_rx) = tokio::sync::oneshot::channel::<MangoHudAlignment>();
        let mangohud_rx = fused_mangohud_alignment_receiver(mangohud_rx);
        tokio::pin!(mangohud_rx);
        let (frame_tx, mut frame_rx) = tokio::sync::mpsc::channel(1024);

        if let Some(run) = crate::recorder_mut!(self.handles).run.as_ref()
            && let Some(path) = self.config.mangohud.log.clone()
            && let Some(offset) = run.mangohud_start_offset
        {
            let path_clone = path.clone();
            let alignment_poll = Duration::from_millis(self.config.mangohud.alignment_poll_ms);
            let tail_idle_sleep = Duration::from_millis(self.config.mangohud.tail_idle_sleep_ms);
            tokio::spawn(async move {
                if let Ok(res) = mangohud::poll_alignment(&path_clone, offset, alignment_poll).await
                {
                    let _ = mangohud_tx.send(res);
                }
            });

            if self.config.mangohud.log_live {
                tokio::spawn(async move {
                    if let Err(err) =
                        mangohud::tail_frames(path, offset, frame_tx, tail_idle_sleep).await
                    {
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

                _ = optional_tick(dmabuf_tick.as_mut()) => {
                    self.handle_dmabuf_log_tick().await?;
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

    pub(super) async fn drain_bpf_events(&mut self) -> anyhow::Result<()> {
        let mut guard = self.handles.ebpf.loaded.events.readable_mut().await?;
        let recording_monotonic_start_ns = crate::recorder!(self.handles)
            .run
            .as_ref()
            .and_then(|r| r.monotonic_start_ns);

        let mut pending_spikes = Vec::new();
        let mut pending_irqs = Vec::new();
        let mut pending_ios = Vec::new();
        let mut pending_monitor_events = Vec::new();

        let current_scx = scx_snapshot(&self.runtime.probes.scx_tracker);

        while let Some(item) = guard.get_inner_mut().next() {
            let Some(kind) = crate::events::decode::read_u32_unaligned(&item) else {
                log::warn!("short_bpf_event len={}", item.len());
                continue;
            };
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
                            self.handles.ebpf.loaded.block_io_correlation_basis.as_str(),
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
                        let source = kms_flip_provider_name(event.provider).to_owned();
                        let driver = display_driver_from_source(&source);
                        let card = (event.card_minor != 0)
                            .then(|| format!("card{}", event.card_minor))
                            .or_else(|| self.config.kms_timing.drm_card.clone())
                            .or_else(|| self.config.drm_fence.display_card.clone());
                        let connector = self
                            .config
                            .kms_timing
                            .connector
                            .clone()
                            .or_else(|| self.config.display_path.connector.clone());
                        pending_monitor_events.push(
                            crate::session_events::MonitorEvent::KmsFlipEvent {
                                event: Box::new(crate::recorder::KmsFlipEventRecord {
                                    elapsed_ms,
                                    timestamp_ns: event.timestamp_ns,
                                    source,
                                    card,
                                    driver,
                                    crtc_id: (event.flags & stutter_common::KMS_FLIP_HAS_CRTC != 0)
                                        .then_some(event.crtc_id),
                                    connector,
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
                        let source = drm_fence_provider_name(event.provider).to_owned();
                        let gpu_role = drm_gpu_role_name(event.gpu_role).to_owned();
                        let driver = display_driver_from_source(&source);
                        let card = match gpu_role.as_str() {
                            "render" => self.config.drm_fence.render_card.clone(),
                            "display" => self.config.drm_fence.display_card.clone(),
                            _ => None,
                        };
                        pending_monitor_events.push(
                            crate::session_events::MonitorEvent::DrmFenceEvent {
                                event: Box::new(crate::recorder::DrmFenceEventRecord {
                                    elapsed_ms,
                                    timestamp_ns: event.timestamp_ns,
                                    source,
                                    event_kind: drm_fence_event_kind_name(event.event_kind)
                                        .to_owned(),
                                    driver,
                                    card,
                                    gpu_role: Some(gpu_role),
                                    // WAIT_DONE_WITHOUT_START means pid/tid are intentionally absent
                                    // even though the event is classified as importer-side.
                                    pid: (event.flags & stutter_common::DRM_FENCE_HAS_PID != 0)
                                        .then_some(event.pid.into()),
                                    tid: (event.flags & stutter_common::DRM_FENCE_HAS_PID != 0)
                                        .then_some(event.tid.into()),
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
                                    signal_ns: (event.signal_ns != 0).then_some(event.signal_ns),
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
}
