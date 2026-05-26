use super::*;

impl MonitorSession {
    pub(crate) async fn handle_probe_drain(
        &mut self,
        _context: ProbeDrainContext,
    ) -> anyhow::Result<()> {
        self.drain_bpf_events().await
    }

    pub(crate) async fn handle_wayland_presentation_tick(
        &mut self,
        context: WaylandPresentationTickContext,
    ) -> anyhow::Result<()> {
        self.dispatch_monitor_event(MonitorEvent::WaylandPresentationEvent {
            event: Box::new(context.event),
        })
        .await
    }

    pub(crate) async fn handle_dmabuf_tick(
        &mut self,
        context: DmaBufTickContext,
    ) -> anyhow::Result<()> {
        self.dispatch_monitor_event(MonitorEvent::DmaBufEvent {
            event: Box::new(context.event),
        })
        .await
    }

    pub(crate) fn normalize_wayland_presentation_event(
        &self,
        mut event: recorder::WaylandPresentationEventRecord,
    ) -> recorder::WaylandPresentationEventRecord {
        let timestamp_ns = event.presented_ns.or(event.commit_ns);
        if let (Some(start_ns), Some(timestamp_ns)) = (
            crate::recorder!(self.handles)
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

    pub(crate) fn normalize_dmabuf_event(
        &self,
        mut event: recorder::DmaBufEventRecord,
    ) -> recorder::DmaBufEventRecord {
        if event.elapsed_ms == 0 {
            event.elapsed_ms = self.started.elapsed().as_millis() as u64;
        }
        event
    }

    pub(crate) async fn handle_wayland_presentation_log_tick(&mut self) -> anyhow::Result<()> {
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

    pub(crate) async fn handle_dmabuf_log_tick(&mut self) -> anyhow::Result<()> {
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
}
