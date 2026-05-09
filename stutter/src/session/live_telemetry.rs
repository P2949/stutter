use std::collections::VecDeque;

use crate::{
    diagnosis::LiveDiagnosisEntry,
    recorder::{BlockIoRecord, FrameEvent, GpuSample, IrqEventRecord, SpikeEvent},
};

pub struct LiveTelemetry {
    pub spikes: VecDeque<SpikeEvent>,
    pub irq_events: VecDeque<IrqEventRecord>,
    pub gpu_samples: VecDeque<GpuSample>,
    pub io_events: VecDeque<BlockIoRecord>,
    pub frame_events: VecDeque<FrameEvent>,
    pub diagnoses: VecDeque<LiveDiagnosisEntry>,
    pub max_age_ms: u128,
}

impl LiveTelemetry {
    pub fn push_spike(&mut self, event: SpikeEvent) {
        self.spikes.push_back(event);
    }

    pub fn push_irq(&mut self, event: IrqEventRecord) {
        self.irq_events.push_back(event);
    }

    pub fn push_gpu(&mut self, sample: GpuSample) {
        self.gpu_samples.push_back(sample);
    }

    pub fn push_io(&mut self, event: BlockIoRecord) {
        self.io_events.push_back(event);
    }

    pub fn push_frame(&mut self, event: FrameEvent) {
        self.frame_events.push_back(event);
    }

    pub fn prune(&mut self, now_ms: u128) {
        while self.spikes.front().is_some_and(|s| {
            now_ms.saturating_sub(s.elapsed_ms.unwrap_or(0).into()) > self.max_age_ms
        }) {
            self.spikes.pop_front();
        }

        while self.irq_events.front().is_some_and(|e| {
            now_ms.saturating_sub(e.elapsed_ms.unwrap_or(0).into()) > self.max_age_ms
        }) {
            self.irq_events.pop_front();
        }

        while self
            .gpu_samples
            .front()
            .is_some_and(|s| now_ms.saturating_sub(s.elapsed_ms.into()) > self.max_age_ms)
        {
            self.gpu_samples.pop_front();
        }

        while self
            .io_events
            .front()
            .is_some_and(|e| now_ms.saturating_sub(e.elapsed_ms.into()) > self.max_age_ms)
        {
            self.io_events.pop_front();
        }

        while self
            .frame_events
            .front()
            .is_some_and(|e| now_ms.saturating_sub(e.elapsed_ms.into()) > self.max_age_ms)
        {
            self.frame_events.pop_front();
        }

        while self
            .diagnoses
            .front()
            .is_some_and(|d| now_ms.saturating_sub(d.elapsed_ms.into()) > self.max_age_ms)
        {
            self.diagnoses.pop_front();
        }
    }
}

impl Default for LiveTelemetry {
    fn default() -> Self {
        Self {
            spikes: VecDeque::new(),
            irq_events: VecDeque::new(),
            gpu_samples: VecDeque::new(),
            io_events: VecDeque::new(),
            frame_events: VecDeque::new(),
            diagnoses: VecDeque::new(),
            max_age_ms: 10_000,
        }
    }
}
