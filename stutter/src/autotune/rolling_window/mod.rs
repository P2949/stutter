mod model;
mod score;
mod signals;
mod utils;

use std::{collections::VecDeque, time::Duration};

pub use model::RollingWindowScore;
use utils::*;

use crate::{
    autotune::{objective::ObjectiveSignals, quality::OnlineDataQualityPolicy},
    diagnosis::LiveDiagnosisEntry,
    recorder::{
        BlockIoRecord, CpuFreqRecord, ForegroundEvent, FrameEvent, GpuSample, IntervalRecord,
        IrqEventRecord,
    },
};

#[derive(Debug, Clone)]
pub struct RollingWindow {
    duration: Duration,
    intervals: VecDeque<IntervalRecord>,
    frames: VecDeque<FrameEvent>,
    diagnoses: VecDeque<LiveDiagnosisEntry>,
    irq_events: VecDeque<IrqEventRecord>,
    block_io_events: VecDeque<BlockIoRecord>,
    gpu_samples: VecDeque<GpuSample>,
    cpu_freq_events: VecDeque<CpuFreqRecord>,
    foreground_events: VecDeque<ForegroundEvent>,
    dropped_invalid_frames: u64,
}

impl RollingWindow {
    pub fn new(duration: Duration) -> Self {
        Self {
            duration,
            intervals: VecDeque::new(),
            frames: VecDeque::new(),
            diagnoses: VecDeque::new(),
            irq_events: VecDeque::new(),
            block_io_events: VecDeque::new(),
            gpu_samples: VecDeque::new(),
            cpu_freq_events: VecDeque::new(),
            foreground_events: VecDeque::new(),
            dropped_invalid_frames: 0,
        }
    }

    pub fn intervals(&self) -> &VecDeque<IntervalRecord> {
        &self.intervals
    }
    pub fn frames(&self) -> &VecDeque<FrameEvent> {
        &self.frames
    }
    pub fn diagnoses(&self) -> &VecDeque<LiveDiagnosisEntry> {
        &self.diagnoses
    }
    pub fn irq_events(&self) -> &VecDeque<IrqEventRecord> {
        &self.irq_events
    }
    pub fn block_io_events(&self) -> &VecDeque<BlockIoRecord> {
        &self.block_io_events
    }
    pub fn gpu_samples(&self) -> &VecDeque<GpuSample> {
        &self.gpu_samples
    }
    pub fn cpu_freq_events(&self) -> &VecDeque<CpuFreqRecord> {
        &self.cpu_freq_events
    }
    pub fn foreground_events(&self) -> &VecDeque<ForegroundEvent> {
        &self.foreground_events
    }

    pub fn duration(&self) -> Duration {
        self.duration
    }

    pub fn duration_ms(&self) -> u64 {
        self.duration.as_millis().min(u128::from(u64::MAX)) as u64
    }

    pub fn is_empty(&self) -> bool {
        self.intervals.is_empty()
            && self.frames.is_empty()
            && self.diagnoses.is_empty()
            && self.irq_events.is_empty()
            && self.block_io_events.is_empty()
            && self.gpu_samples.is_empty()
            && self.cpu_freq_events.is_empty()
            && self.foreground_events.is_empty()
    }

    pub fn clear(&mut self) {
        self.intervals.clear();
        self.frames.clear();
        self.diagnoses.clear();
        self.irq_events.clear();
        self.block_io_events.clear();
        self.gpu_samples.clear();
        self.cpu_freq_events.clear();
        self.foreground_events.clear();
        self.dropped_invalid_frames = 0;
    }

    pub fn latest_elapsed_ms(&self) -> Option<u64> {
        [
            self.intervals.back().map(|record| record.elapsed_ms),
            self.frames.back().map(|frame| frame.elapsed_ms),
            self.diagnoses.back().map(|diagnosis| diagnosis.elapsed_ms),
            self.irq_events.back().and_then(|event| event.elapsed_ms),
            self.block_io_events.back().map(|event| event.elapsed_ms),
            self.gpu_samples.back().map(|sample| sample.elapsed_ms),
            self.cpu_freq_events.back().map(|event| event.elapsed_ms),
            self.foreground_events.back().map(|event| event.elapsed_ms),
        ]
        .into_iter()
        .flatten()
        .max()
    }

    pub fn window_start_ms_for(&self, now_elapsed_ms: u64) -> u64 {
        now_elapsed_ms.saturating_sub(self.duration_ms())
    }

    pub fn push_interval(&mut self, record: IntervalRecord) {
        let elapsed_ms = record.elapsed_ms;
        push_sorted_by_elapsed(&mut self.intervals, record, |record| record.elapsed_ms);
        self.prune_to(elapsed_ms);
    }

    pub fn push_intervals<I>(&mut self, records: I)
    where
        I: IntoIterator<Item = IntervalRecord>,
    {
        let mut records = records.into_iter().collect::<Vec<_>>();
        if records.is_empty() {
            return;
        }

        records.sort_by_key(|record| record.elapsed_ms);
        self.intervals.extend(records);
        sort_intervals_by_elapsed(&mut self.intervals);
        // invariant: self.intervals is non-empty because the empty batch returned above.
        let latest_elapsed_ms = self.intervals[self.intervals.len() - 1].elapsed_ms;
        self.prune_to(latest_elapsed_ms);
    }

    pub fn push_frame(&mut self, frame: FrameEvent) {
        let elapsed_ms = frame.elapsed_ms;
        if !is_valid_frametime_ms(frame.frametime_ms) {
            self.dropped_invalid_frames = self.dropped_invalid_frames.saturating_add(1);
            self.prune_to(elapsed_ms);
            return;
        }

        push_sorted_by_elapsed(&mut self.frames, frame, |frame| frame.elapsed_ms);
        self.prune_to(elapsed_ms);
    }

    pub fn push_irq_event(&mut self, mut event: IrqEventRecord) {
        let Some(elapsed_ms) = event.elapsed_ms.or_else(|| self.latest_elapsed_ms()) else {
            return;
        };
        event.elapsed_ms = Some(elapsed_ms);
        push_sorted_by_elapsed(&mut self.irq_events, event, |event| {
            event.elapsed_ms.unwrap_or(0)
        });
        self.prune_to(elapsed_ms);
    }

    pub fn push_block_io_event(&mut self, event: BlockIoRecord) {
        let elapsed_ms = event.elapsed_ms;
        push_sorted_by_elapsed(&mut self.block_io_events, event, |event| event.elapsed_ms);
        self.prune_to(elapsed_ms);
    }

    pub fn push_gpu_sample(&mut self, sample: GpuSample) {
        let elapsed_ms = sample.elapsed_ms;
        push_sorted_by_elapsed(&mut self.gpu_samples, sample, |sample| sample.elapsed_ms);
        self.prune_to(elapsed_ms);
    }

    pub fn push_cpu_freq_event(&mut self, event: CpuFreqRecord) {
        let elapsed_ms = event.elapsed_ms;
        push_sorted_by_elapsed(&mut self.cpu_freq_events, event, |event| event.elapsed_ms);
        self.prune_to(elapsed_ms);
    }

    pub fn push_foreground_event(&mut self, event: ForegroundEvent) {
        let elapsed_ms = event.elapsed_ms;
        push_sorted_by_elapsed(&mut self.foreground_events, event, |event| event.elapsed_ms);
        self.prune_to(elapsed_ms);
    }

    pub fn push_diagnosis(&mut self, diagnosis: LiveDiagnosisEntry) {
        let elapsed_ms = diagnosis.elapsed_ms;
        push_sorted_by_elapsed(&mut self.diagnoses, diagnosis, |diagnosis| {
            diagnosis.elapsed_ms
        });
        self.prune_to(elapsed_ms);
    }

    pub fn prune_to(&mut self, now_elapsed_ms: u64) {
        let start_ms = self.window_start_ms_for(now_elapsed_ms);

        drain_front_before_elapsed(&mut self.intervals, start_ms, |record| record.elapsed_ms);
        drain_front_before_elapsed(&mut self.frames, start_ms, |frame| frame.elapsed_ms);
        drain_front_before_elapsed(&mut self.diagnoses, start_ms, |diagnosis| {
            diagnosis.elapsed_ms
        });
        drain_front_before_elapsed(&mut self.irq_events, start_ms, |event| {
            event.elapsed_ms.unwrap_or(0)
        });
        drain_front_before_elapsed(&mut self.block_io_events, start_ms, |event| {
            event.elapsed_ms
        });
        drain_front_before_elapsed(&mut self.gpu_samples, start_ms, |sample| sample.elapsed_ms);
        drain_front_before_elapsed(&mut self.cpu_freq_events, start_ms, |event| {
            event.elapsed_ms
        });
        drain_front_before_elapsed(&mut self.foreground_events, start_ms, |event| {
            event.elapsed_ms
        });
    }

    pub fn retain_latest_window(&mut self) {
        if let Some(now_elapsed_ms) = self.latest_elapsed_ms() {
            self.prune_to(now_elapsed_ms);
        }
    }

    pub fn interval_count(&self) -> usize {
        self.intervals.len()
    }

    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    pub fn dropped_invalid_frame_count(&self) -> u64 {
        self.dropped_invalid_frames
    }

    pub fn diagnosis_count(&self) -> usize {
        self.diagnoses.len()
    }

    pub fn total_event_count(&self) -> usize {
        self.interval_count()
            .saturating_add(self.frame_count())
            .saturating_add(self.diagnosis_count())
            .saturating_add(self.irq_events.len())
            .saturating_add(self.block_io_events.len())
            .saturating_add(self.gpu_samples.len())
            .saturating_add(self.cpu_freq_events.len())
            .saturating_add(self.foreground_events.len())
    }

    pub fn frame_p99_ms(&self) -> f64 {
        percentile_f64(
            self.frames
                .iter()
                .map(|frame| frame.frametime_ms)
                .filter(|value| is_valid_frametime_ms(*value))
                .collect(),
            0.99,
        )
    }

    pub fn frame_max_ms(&self) -> f64 {
        self.frames
            .iter()
            .map(|frame| frame.frametime_ms)
            .filter(|value| is_valid_frametime_ms(*value))
            .fold(0.0, f64::max)
    }

    pub fn scored_samples(&self) -> u64 {
        self.intervals
            .iter()
            .map(|record| record.samples)
            .fold(0_u64, u64::saturating_add)
    }

    pub fn recent_diagnoses_vec(&self) -> Vec<LiveDiagnosisEntry> {
        self.diagnoses.iter().cloned().collect()
    }

    pub fn objective_signals(&self) -> ObjectiveSignals {
        signals::compute_objective_signals(self)
    }

    pub fn score(&self) -> RollingWindowScore {
        self.score_with_quality_policy(&OnlineDataQualityPolicy::default())
    }

    pub(crate) fn score_with_quality_policy(
        &self,
        quality_policy: &OnlineDataQualityPolicy,
    ) -> RollingWindowScore {
        score::compute_rolling_window_score(self, quality_policy)
    }
}

impl Default for RollingWindow {
    fn default() -> Self {
        Self::new(Duration::from_secs(30))
    }
}

#[cfg(test)]
mod tests;
