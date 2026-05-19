use std::sync::Arc;

use super::{
    ForegroundEvent, GpuSample, IntervalCsvWriter, IntervalRecord, IrqEventRecord, RecordingRun,
    SpikeEvent, SpikeEventBuffer, SpikePushResult, StdoutJsonStream, TreeEvent,
};
use crate::{artifacts::ArtifactStreamRegistry, prometheus::PrometheusState};

#[derive(Default, Debug)]
pub struct LiveBuffers {
    pub interval_records: Vec<IntervalRecord>,
    pub tree_events: Vec<TreeEvent>,
    pub spike_events: Option<SpikeEventBuffer>,
    pub irq_events: Vec<IrqEventRecord>,
    pub gpu_samples: Vec<GpuSample>,
    pub scx_events: Vec<crate::scx::ScxEvent>,
}

#[derive(Default, Debug)]
pub struct RecordingCounters {
    pub intervals_dropped: u64,
    pub scx_event_count: u64,
    pub irq_event_count: u64,
    pub migration_event_count: u64,
    pub cpu_freq_sample_count: u64,
    pub gpu_sample_count: u64,
    pub block_io_event_count: u64,
    pub runtime_slice_count: u64,
    pub runtime_slice_read_errors: u64,
    pub runtime_slice_skipped_tasks: u64,
    pub interval_record_count: u64,
    pub frame_event_count: u64,
    pub focus_event_count: u64,
    pub foreground_event_count: u64,
    pub kms_flip_event_count: u64,
    pub drm_fence_event_count: u64,
    pub wayland_presentation_event_count: u64,
    pub process_scan_budget_exceeded_count: u64,
    pub thread_scan_limited_count: u64,

    pub spike_event_count: u64,
    pub spike_events_dropped_count: u64,
    pub alert_events_dropped_count: u64,
    pub alert_channel_closed_count: u64,

    pub event_stream_write_errors: u64,
    pub first_event_stream_write_error: Option<String>,

    pub stdout_spike_stream_errors: u64,
}

#[derive(Default, Debug)]
pub struct ExporterState {
    pub prometheus_state: Option<Arc<PrometheusState>>,
    pub otel_spike_tx: Option<tokio::sync::mpsc::Sender<crate::otel::OtelSpike>>,
    pub otel_spans_dropped: Option<Arc<std::sync::atomic::AtomicU64>>,
}

#[derive(Default)]
pub struct LiveRecorder {
    pub run: Option<RecordingRun>,
    pub buffers: LiveBuffers,
    pub streams: ArtifactStreamRegistry,
    pub csv_writer: Option<IntervalCsvWriter>,
    pub stdout_spike_stream: Option<StdoutJsonStream>,
    pub counters: RecordingCounters,
    pub exporters: ExporterState,
    pub last_foreground_event: Option<ForegroundEvent>,
}

impl std::fmt::Debug for LiveRecorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveRecorder")
            .field("run", &self.run)
            .field("buffers", &self.buffers)
            .field("counters", &self.counters)
            .field("exporters", &self.exporters)
            .field("last_foreground_event", &self.last_foreground_event)
            .finish_non_exhaustive()
    }
}

impl RecordingCounters {
    pub fn record_stream_write_error<E: std::fmt::Display>(&mut self, stream_name: &str, err: E) {
        self.event_stream_write_errors += 1;
        if self.first_event_stream_write_error.is_none() {
            self.first_event_stream_write_error = Some(format!("{stream_name}: {err}"));
        }
    }
}

impl LiveRecorder {
    pub fn push_spike_event_to_buffer(&mut self, spike_event: SpikeEvent) {
        if let Some(spike_events) = self.buffers.spike_events.as_mut() {
            match spike_events.push(spike_event) {
                SpikePushResult::Stored => {}
                SpikePushResult::Dropped => {
                    self.counters.spike_events_dropped_count += 1;
                }
            }
        }
    }

    pub fn enable_stdout_spike_stream(&mut self) {
        self.stdout_spike_stream = Some(StdoutJsonStream::new());
    }

    #[cfg(test)]
    pub fn write_foreground_event(&mut self, event: ForegroundEvent) -> anyhow::Result<()> {
        use crate::session::sinks::{
            MonitorEventSink, MonitorOutputConfig, MonitorSinkContext, RecorderSink,
        };

        let event = crate::session_events::MonitorEvent::ForegroundEvent {
            event: Box::new(event),
        };
        let mut ctx = MonitorSinkContext {
            recorder: self,
            alert_sender: None,
            output: MonitorOutputConfig::default(),
        };
        let mut sink = RecorderSink::new();

        sink.on_event(&event, &mut ctx)
            .map_err(|err| anyhow::anyhow!(err))
    }
}
