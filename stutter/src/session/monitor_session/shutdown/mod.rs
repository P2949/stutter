use super::*;

/// A trait for flushing recorder streams, allowing test fakes to implement it.
pub(crate) trait RecorderFlush {
    fn flush_streams(&mut self) -> anyhow::Result<()>;
}

impl RecorderFlush for crate::recorder::LiveRecorder {
    fn flush_streams(&mut self) -> anyhow::Result<()> {
        self.streams.finish_all()
    }
}

/// Executes the shutdown sequence in explicit order:
///
/// 1. Stop event ingestion (drop the bus).
/// 2. Flush recorder/exporters (flush streams).
/// 3. Detach probes (drop exporters, then ebpf).
/// 4. Final report (call `final_report` closure).
pub(crate) fn execute_shutdown_sequence<B, E, X, R: RecorderFlush>(
    bus: B,
    recorder: &mut R,
    exporters: X,
    ebpf: E,
    final_report: impl FnOnce(&mut R) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    // 1. Stop event ingestion.
    drop(bus);
    // 2. Flush recorder/exporters.
    recorder.flush_streams()?;
    // 3. Detach probes.
    drop(exporters);
    drop(ebpf);
    // 4. Final report.
    final_report(recorder)
}

impl MonitorSession {
    pub(crate) fn handle_ctrl_c_stop(&self) -> String {
        "ctrl_c".to_owned()
    }

    pub(crate) fn handle_max_duration_stop(&self, reason: Option<String>) -> String {
        reason.unwrap_or_else(|| "max_duration".to_owned())
    }

    pub(crate) fn handle_remote_stop(&self) -> String {
        "remote_stop".to_owned()
    }

    pub(crate) fn handle_epoch_tick(&self) -> Option<String> {
        self.config
            .timing
            .epoch_period_ms
            .is_some()
            .then(|| "epoch_ended".to_owned())
    }

    pub fn finalize(mut self, stop_reason: String) -> anyhow::Result<String> {
        if let Some(term) = self.runtime.ui.terminal.as_mut() {
            let _ = crate::tui::restore_terminal(term);
        }

        let drop_counters = self.handles.ebpf.loaded.snapshot_drop_counters();
        log_drop_counters(&drop_counters);

        if let Some(dropped) = crate::recorder_mut!(self.handles)
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

        if let Some(writer) = crate::recorder_mut!(self.handles).csv_writer.as_mut() {
            writer.finish()?;
            if let Some(CsvStreamTarget::File(path)) = &self.config.streams.csv
                && !self.config.outputs.json_stream
            {
                println!("wrote interval CSV: {}", path.display());
            }
        }

        if crate::recorder!(self.handles).run.is_some() {
            let frame_events = crate::session::mangohud_frames::read_and_stream_non_live_events(
                self.config.as_ref(),
                &mut crate::recorder_mut!(self.handles),
            );

            let probe_activation_warnings = self.handles.ebpf.recorded_activation_warnings();

            // Extract fields before moving parts of self.
            let config = self.config.clone();
            let current_focus = self.current_focus.clone();
            let current_foreground = self.current_foreground.clone();
            let focus_switch_count = self.focus_switch_count;
            let tree_pids = self.runtime.targeting.effective_tree_pids().to_vec();
            let tasks_ref = &self.runtime.targeting.tasks;
            // SAFETY: borrow is needed for the closure; but we can't capture it by ref across a closure bound nicely.
            // Instead, capture a snapshot of relevant fields.
            let block_io_basis = self.runtime.probes.block_io_correlation_basis.clone();
            let block_io_confidence = self.runtime.probes.block_io_correlation_confidence.clone();
            let native_cgroup_filter = self.runtime.probes.native_cgroup_filter.clone();
            let cpu_perf_status = self
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
                });
            let focus_mode = if config.focus.auto_focus {
                Some("auto".to_owned())
            } else if config.has_explicit_target() {
                Some("explicit".to_owned())
            } else {
                Some("legacy-auto-detect".to_owned())
            };
            let final_focus_kind = current_focus
                .as_ref()
                .map(|focus| format!("{:?}", focus.group.kind));
            let foreground_include_title = config.focus.foreground_include_title;

            let final_report_fn =
                |live_recorder: &mut crate::recorder::LiveRecorder| -> anyhow::Result<()> {
                    let final_foreground_event = foreground_event_for_final_metadata(
                        current_foreground.as_ref(),
                        live_recorder.last_foreground_event.as_ref(),
                        foreground_include_title,
                    );
                    recorder::finalize_recording(FinalizeRecordingInput {
                        recorder: live_recorder,
                        config: &config,
                        tree_pids: &tree_pids,
                        stop_reason: &stop_reason,
                        tasks: tasks_ref,
                        frame_events: &frame_events,
                        block_io_correlation_basis: block_io_basis,
                        block_io_correlation_confidence: block_io_confidence,
                        native_cgroup_filter,
                        probe_activation_warnings,
                        focus_mode,
                        final_focus_kind,
                        focus_switch_count,
                        current_focus,
                        final_foreground_event,
                        drop_counters,
                        cpu_perf_status,
                    })?;

                    recorder::print_recording_warnings(live_recorder);
                    Ok(())
                };

            let bus = self.runtime.bus;
            let exporters = self.handles.exporters;
            let ebpf = self.handles.ebpf;
            // invariant: recorder is populated during run
            let mut live_recorder = self.handles.recorder.take().unwrap().recorder;

            execute_shutdown_sequence(bus, &mut live_recorder, exporters, ebpf, final_report_fn)?;
        }

        info!("exiting stop_reason={stop_reason}");
        Ok(stop_reason)
    }
}

#[cfg(test)]
mod tests;
