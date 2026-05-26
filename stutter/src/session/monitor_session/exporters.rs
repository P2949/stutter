use super::*;

impl MonitorSession {
    pub fn emit(
        &mut self,
        event: MonitorEvent,
    ) -> impl std::future::Future<Output = ()> + Send + '_ {
        self.runtime.bus.emit(event)
    }

    pub(crate) async fn dispatch_monitor_event(
        &mut self,
        event: MonitorEvent,
    ) -> anyhow::Result<()> {
        let output = self.runtime.event_runtime_config.output;
        let outputs = &mut self.runtime.outputs;
        let mut sinks = MonitorOutputSinks::new(
            output,
            &mut crate::recorder_mut!(self.handles),
            outputs.alert_sender.as_ref(),
            &mut outputs.sink_registry,
        );

        if let Err(err) = sinks.dispatch(&event) {
            warn!("monitor_event_sink_failed err={err}");
        }

        self.emit(event).await;
        Ok(())
    }

    pub(crate) async fn handle_focus_context_tick(
        &mut self,
        _context: FocusTickContext,
    ) -> anyhow::Result<()> {
        self.handle_focus_tick().await
    }

    pub(crate) async fn handle_foreground_context_tick(
        &mut self,
        _context: ForegroundTickContext,
    ) -> anyhow::Result<()> {
        self.handle_foreground_tick().await
    }

    pub(crate) async fn handle_summary_context_tick(
        &mut self,
        _context: SummaryTickContext,
    ) -> anyhow::Result<()> {
        self.handle_summary_tick().await
    }

    pub(crate) async fn handle_frame_tick(
        &mut self,
        context: FrameTickContext,
    ) -> anyhow::Result<()> {
        let frame = context.frame;
        self.dispatch_monitor_event(MonitorEvent::Frame {
            event: Box::new(frame.clone()),
        })
        .await?;
        self.runtime.telemetry.push_frame(frame);
        Ok(())
    }

    pub(crate) async fn handle_telemetry_tick(
        &mut self,
        context: TelemetryTickContext,
    ) -> anyhow::Result<()> {
        match context.event {
            TelemetryTickEvent::MangoHudAlignment {
                raw_ms,
                monotonic_ns,
            } => {
                if let Some(run) = crate::recorder_mut!(self.handles).run.as_mut() {
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

    pub async fn handle_summary_tick(&mut self) -> anyhow::Result<()> {
        if !self.runtime.ui.tui_state.paused {
            let elapsed_ms = self.started.elapsed().as_millis() as u64;
            if let Some(sampler) = self.runtime.probes.cpu_perf_sampler.as_mut() {
                let deltas = sampler.sample_interval();
                for (tid, delta) in deltas {
                    if let Some(stats) = self
                        .runtime
                        .targeting
                        .tasks
                        .stats_by_task
                        .get_mut(&tid.as_u32())
                    {
                        stats.record_cpu_perf(&delta);
                    }
                }
            }

            let drop_counters_snapshot = self.handles.ebpf.loaded.snapshot_drop_counters();
            let psi_snapshot = self.runtime.probes.psi_reader.read_with_delta().ok();
            let records = collect_interval_summaries_labeled(
                self.interval_label,
                &mut self.runtime.targeting.tasks.stats_by_task,
                elapsed_ms,
                &drop_counters_snapshot,
                self.handles.ebpf.loaded.prev_faults_map.as_ref(),
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
                crate::recorder_mut!(self.handles)
                    .counters
                    .runtime_slice_read_errors = crate::recorder_mut!(self.handles)
                    .counters
                    .runtime_slice_read_errors
                    .saturating_add(batch.read_errors);
                crate::recorder_mut!(self.handles)
                    .counters
                    .runtime_slice_skipped_tasks = crate::recorder_mut!(self.handles)
                    .counters
                    .runtime_slice_skipped_tasks
                    .saturating_add(batch.skipped_tasks as u64);

                if crate::recorder_mut!(self.handles)
                    .streams
                    .contains(ArtifactKind::RuntimeSlices)
                {
                    for record in &batch.records {
                        crate::artifacts::push_artifact_event(
                            &mut crate::recorder_mut!(self.handles),
                            ArtifactKind::RuntimeSlices,
                            record,
                            "runtime_slices",
                            |c| {
                                c.runtime_slice_count += 1;
                            },
                        );
                    }
                } else {
                    crate::recorder_mut!(self.handles)
                        .counters
                        .runtime_slice_count = crate::recorder_mut!(self.handles)
                        .counters
                        .runtime_slice_count
                        .saturating_add(batch.records.len() as u64);
                }
            }

            if let Some(state) = self.handles.exporters.prometheus_state.as_ref() {
                let max_p99 = records.iter().map(|r| r.p99_ns).max().unwrap_or(0);
                state.set_latest_p99_ns(max_p99);
                state.set_active_targets(self.runtime.targeting.tasks.active_targets.len() as u64);
                state.set_event_stream_write_errors(
                    crate::recorder_mut!(self.handles)
                        .counters
                        .event_stream_write_errors,
                );
                state.set_ebpf_ringbuf_drops(drop_counters_snapshot.total());
            }
        }

        if let Some(term) = self.runtime.ui.terminal.as_mut() {
            let elapsed_ms = self.started.elapsed().as_millis() as u64;
            let drop_counters_snapshot = self.handles.ebpf.loaded.snapshot_drop_counters();

            let snapshot = TuiRenderSnapshot {
                elapsed_ms,
                drop_counters: drop_counters_snapshot,
                tui_state: self.runtime.ui.tui_state.clone(),
                active_targets: self.runtime.targeting.tasks.active_targets.clone(),
                stats_by_task: self.runtime.targeting.tasks.stats_by_task.clone(),
                interval_records: crate::recorder!(self.handles)
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

    pub(crate) async fn handle_live_spike(&mut self, spike: SpikeEvent) -> anyhow::Result<()> {
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
            intervals: crate::recorder_mut!(self.handles)
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
}
