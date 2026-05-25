use std::{
    collections::{BTreeSet, VecDeque},
    time::Duration,
};

use crate::{
    autotune::{
        objective::{ObjectiveSignalQuality, ObjectiveSignalQualitySnapshot, ObjectiveSignals},
        quality::{OnlineDataQuality, OnlineDataQualityInput, OnlineDataQualityPolicy},
    },
    diagnosis::LiveDiagnosisEntry,
    recorder::{
        BlockIoRecord, CpuFreqRecord, ForegroundEvent, FrameEvent, GpuSample, IntervalRecord,
        IrqEventRecord,
    },
};

#[derive(Debug, Clone)]
pub struct RollingWindowScore {
    pub duration_ms: u64,
    pub interval_count: usize,
    pub scored_task_count: usize,
    pub scored_samples: u64,
    pub diagnostic_score_total: u64,
    pub over_1ms: u64,
    pub over_2ms: u64,
    pub over_5ms: u64,
    pub max_latency_ns: u64,
    pub frame_count: usize,
    pub frame_p99_ms: f64,
    pub frame_max_ms: f64,
    pub dropped_invalid_frames: u64,
    pub data_quality: OnlineDataQuality,
}

const GPU_THERMAL_DEGRADED_MILLIDEGREES: u32 = 85_000;
const GPU_POWER_LIMIT_BUSY_PERCENT: u32 = 95;
const GPU_POWER_LIMIT_LOW_CLOCK_MHZ: u32 = 300;

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
        let block_io_overlap_count = self
            .block_io_events
            .iter()
            .filter(|event| event.duration_ns > 0)
            .count() as u64;
        let block_io_worst_latency_ns = self
            .block_io_events
            .iter()
            .map(|event| event.duration_ns)
            .max()
            .unwrap_or(0);
        let dirty_writeback_events = self
            .block_io_events
            .iter()
            .filter(|event| event.rwbs.contains('W') || event.rwbs.contains('F'))
            .count() as u64;
        let block_io_overlap_basis = overlap_basis_label(
            self.block_io_events
                .iter()
                .map(|event| event.correlation_basis.as_ref()),
        );
        let block_io_quality = source_quality_for_block_io_basis(block_io_overlap_basis.as_deref());
        let has_block_io_events = !self.block_io_events.is_empty();

        let irq_worst_event = self
            .irq_events
            .iter()
            .filter(|event| event.duration_ns > 0)
            .max_by_key(|event| event.duration_ns);
        let irq_overlap_count = self
            .irq_events
            .iter()
            .filter(|event| event.duration_ns > 0)
            .count() as u64;
        let irq_worst_overlap_ns = irq_worst_event.map(|event| event.duration_ns).unwrap_or(0);
        let has_irq_events = !self.irq_events.is_empty();

        let thermal_samples = self
            .gpu_samples
            .iter()
            .filter_map(|sample| sample.temp_millidegrees)
            .collect::<Vec<_>>();
        let thermal_throttle_count = thermal_samples
            .iter()
            .filter(|temp| **temp >= GPU_THERMAL_DEGRADED_MILLIDEGREES)
            .count() as u64;
        let thermal_degraded = (!thermal_samples.is_empty()).then_some(thermal_throttle_count > 0);

        let cpu_power_limited_event = self
            .cpu_freq_events
            .iter()
            .find(|event| event.freq_khz == 0);
        let cpu_power_limited =
            (!self.cpu_freq_events.is_empty()).then_some(cpu_power_limited_event.is_some());

        let latest_gpu = self.gpu_samples.back();
        let gpu_power_limited_event = self.gpu_samples.iter().find(|sample| {
            sample.gpu_busy_percent.unwrap_or(0) >= GPU_POWER_LIMIT_BUSY_PERCENT
                && sample.gpu_clock_mhz.unwrap_or(u32::MAX) <= GPU_POWER_LIMIT_LOW_CLOCK_MHZ
        });
        let gpu_power_limited =
            (!self.gpu_samples.is_empty()).then_some(gpu_power_limited_event.is_some());
        let gpu_power_limit_reason = gpu_power_limited_event
            .and_then(|sample| sample.power_limit_reason.clone())
            .or_else(|| {
                gpu_power_limited_event
                    .is_some()
                    .then(|| "busy_high_clock_low".to_owned())
            });
        let gpu_power_quality = if self.gpu_samples.iter().any(|sample| {
            sample.gpu_busy_percent.is_some()
                || sample.gpu_clock_mhz.is_some()
                || sample.temp_millidegrees.is_some()
                || sample.power_microwatts.is_some()
        }) {
            ObjectiveSignalQuality::Direct
        } else {
            ObjectiveSignalQuality::Missing
        };

        let memory_pressure_some_avg10_percent = (!self.intervals.is_empty()).then(|| {
            let total = self
                .intervals
                .iter()
                .map(|record| record.mem_psi_some.max(0.0))
                .sum::<f64>();
            (total / self.intervals.len() as f64) as f32
        });
        let swap_activity_events = (!self.intervals.is_empty()).then(|| {
            self.intervals
                .iter()
                .map(|record| record.major_faults)
                .fold(0_u64, u64::saturating_add)
        });
        let mem_stall_spike_count = (!self.intervals.is_empty()).then(|| {
            self.intervals
                .iter()
                .map(|record| u64::from(record.mem_psi_spike))
                .fold(0_u64, u64::saturating_add)
        });

        let gpu_active_render_node = latest_gpu.and_then(|sample| sample.render_node.clone());
        let gpu_drm_card = latest_gpu.and_then(|sample| sample.drm_card.clone());

        let signal_quality = ObjectiveSignalQualitySnapshot {
            block_io_overlap: block_io_quality,
            irq_overlap: if has_irq_events {
                ObjectiveSignalQuality::Direct
            } else {
                ObjectiveSignalQuality::Missing
            },
            thermal: if thermal_degraded.is_some() {
                ObjectiveSignalQuality::Direct
            } else {
                ObjectiveSignalQuality::Missing
            },
            cpu_power: if cpu_power_limited.is_some() {
                ObjectiveSignalQuality::Derived
            } else {
                ObjectiveSignalQuality::Missing
            },
            gpu_power: gpu_power_quality,
            gpu_active_render_node: if gpu_active_render_node.is_some() {
                ObjectiveSignalQuality::Direct
            } else {
                ObjectiveSignalQuality::Missing
            },
            memory_pressure: if memory_pressure_some_avg10_percent.is_some() {
                ObjectiveSignalQuality::Direct
            } else {
                ObjectiveSignalQuality::Missing
            },
            swap_activity: if swap_activity_events.is_some() {
                ObjectiveSignalQuality::Approximate
            } else {
                ObjectiveSignalQuality::Missing
            },
            dirty_writeback: if has_block_io_events {
                ObjectiveSignalQuality::Direct
            } else {
                ObjectiveSignalQuality::Missing
            },
            frame_pacing: if self.frames.is_empty() {
                ObjectiveSignalQuality::Missing
            } else {
                ObjectiveSignalQuality::Direct
            },
            foreground_latency: if self.intervals.is_empty() {
                ObjectiveSignalQuality::Missing
            } else {
                ObjectiveSignalQuality::Derived
            },
        };

        let irq_quality = if has_irq_events {
            ObjectiveSignalQuality::Direct
        } else {
            ObjectiveSignalQuality::Missing
        };
        let block_io_overlap_trust =
            has_block_io_events.then(|| block_io_quality.as_str().to_owned());
        let irq_overlap_trust = has_irq_events.then(|| irq_quality.as_str().to_owned());
        let irq_overlap_basis = has_irq_events.then(|| "irq-duration".to_owned());

        ObjectiveSignals {
            block_io_overlap_count: has_block_io_events.then_some(block_io_overlap_count),
            block_io_worst_latency_ns: has_block_io_events.then_some(block_io_worst_latency_ns),
            block_io_overlap_basis,
            block_io_overlap_trust,
            irq_overlap_count: has_irq_events.then_some(irq_overlap_count),
            irq_worst_overlap_ns: has_irq_events.then_some(irq_worst_overlap_ns),
            irq_hot_irq: irq_worst_event.map(|event| event.irq),
            irq_hot_cpu: irq_worst_event.map(|event| event.cpu),
            irq_overlap_basis,
            irq_overlap_trust,
            thermal_degraded,
            thermal_throttle_count: thermal_degraded.map(|_| thermal_throttle_count),
            cpu_power_limited,
            cpu_power_limited_cpu: cpu_power_limited_event.map(|event| event.cpu),
            cpu_power_limit_source: cpu_power_limited_event.map(|_| "cpu_freq_zero_khz".to_owned()),
            cpu_power_limited_policy: cpu_power_limited_event
                .map(|event| format!("cpu{}", event.cpu)),
            gpu_power_limited,
            gpu_power_limit_reason,
            gpu_busy_percent: latest_gpu.and_then(|sample| sample.gpu_busy_percent),
            gpu_clock_mhz: latest_gpu.and_then(|sample| sample.gpu_clock_mhz),
            gpu_temp_millidegrees: latest_gpu.and_then(|sample| sample.temp_millidegrees),
            gpu_drm_card,
            gpu_active_render_node,
            gpu_focus_confidence: latest_gpu
                .and_then(|sample| sample.render_node.as_ref())
                .map(|_| 0.85),
            gpu_focus_source: latest_gpu
                .and_then(|sample| sample.render_node.as_ref())
                .map(|_| "gpu_sample".to_owned()),
            memory_pressure_some_avg10_percent,
            swap_activity_events,
            mem_stall_spike_count,
            dirty_writeback_events: has_block_io_events.then_some(dirty_writeback_events),
            frame_p99_ms: Some(self.frame_p99_ms()),
            foreground_over_5ms: Some(
                self.intervals
                    .iter()
                    .map(|record| record.over_5ms)
                    .fold(0_u64, u64::saturating_add),
            ),
            signal_quality,
        }
    }

    pub fn score(&self) -> RollingWindowScore {
        self.score_with_quality_policy(&OnlineDataQualityPolicy::default())
    }

    pub(crate) fn score_with_quality_policy(
        &self,
        quality_policy: &OnlineDataQualityPolicy,
    ) -> RollingWindowScore {
        let interval_count = self.interval_count();
        let scored_samples = self.scored_samples();
        let over_1ms = self
            .intervals
            .iter()
            .map(|record| record.over_1ms)
            .fold(0_u64, u64::saturating_add);
        let over_2ms = self
            .intervals
            .iter()
            .map(|record| record.over_2ms)
            .fold(0_u64, u64::saturating_add);
        let over_5ms = self
            .intervals
            .iter()
            .map(|record| record.over_5ms)
            .fold(0_u64, u64::saturating_add);
        let max_latency_ns = self
            .intervals
            .iter()
            .map(|record| record.max_ns)
            .max()
            .unwrap_or(0);
        let diagnostic_score_total = over_5ms
            .saturating_mul(100)
            .saturating_add(over_2ms.saturating_mul(20))
            .saturating_add(over_1ms);
        let frame_count = self.frame_count();
        let frame_p99_ms = self.frame_p99_ms();
        let frame_max_ms = self.frame_max_ms();

        let scored_task_count = self
            .intervals
            .iter()
            .filter(|record| record.samples > 0)
            .map(|record| record.task)
            .collect::<BTreeSet<_>>()
            .len();

        let drop_counter_total = self
            .intervals
            .iter()
            .map(|record| record.drop_counters.total())
            .fold(0_u64, u64::saturating_add);

        let data_quality = OnlineDataQualityInput {
            scored_intervals: interval_count,
            scored_samples,
            scored_task_count,
            drop_counter_total,
            target_identity_shifted: false,
            target_present: scored_task_count > 0,
            frame_data_required: false,
            frame_count,
            baseline_frame_count: None,
            candidate_frame_count: None,
            baseline_scored_identity_counts: &[],
            candidate_scored_identity_counts: &[],
        }
        .evaluate_with_policy(quality_policy);

        RollingWindowScore {
            duration_ms: self.duration_ms(),
            interval_count,
            scored_task_count,
            scored_samples,
            diagnostic_score_total,
            over_1ms,
            over_2ms,
            over_5ms,
            max_latency_ns,
            frame_count,
            frame_p99_ms,
            frame_max_ms,
            dropped_invalid_frames: self.dropped_invalid_frame_count(),
            data_quality,
        }
    }
}

impl Default for RollingWindow {
    fn default() -> Self {
        Self::new(Duration::from_secs(30))
    }
}

fn overlap_basis_label<'a>(bases: impl Iterator<Item = &'a str>) -> Option<String> {
    let unique = bases
        .filter(|basis| !basis.trim().is_empty())
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();

    if unique.is_empty() {
        return None;
    }

    Some(unique.into_iter().collect::<Vec<_>>().join("+"))
}

fn source_quality_for_block_io_basis(basis: Option<&str>) -> ObjectiveSignalQuality {
    match basis {
        Some("request-pointer") => ObjectiveSignalQuality::Direct,
        Some(_) => ObjectiveSignalQuality::Approximate,
        None => ObjectiveSignalQuality::Missing,
    }
}

fn drain_front_before_elapsed<T, F>(items: &mut VecDeque<T>, start_ms: u64, elapsed_ms: F)
where
    F: Fn(&T) -> u64,
{
    while items
        .front()
        .is_some_and(|item| elapsed_ms(item) < start_ms)
    {
        items.pop_front();
    }
}

fn push_sorted_by_elapsed<T, F>(items: &mut VecDeque<T>, item: T, elapsed_ms: F)
where
    F: Fn(&T) -> u64,
{
    let item_elapsed_ms = elapsed_ms(&item);

    match items
        .iter()
        .rposition(|existing| elapsed_ms(existing) <= item_elapsed_ms)
    {
        Some(index) => items.insert(index + 1, item),
        None => items.push_front(item),
    }
}

fn sort_intervals_by_elapsed(intervals: &mut VecDeque<IntervalRecord>) {
    let mut records = intervals.drain(..).collect::<Vec<_>>();
    records.sort_by_key(|record| record.elapsed_ms);
    *intervals = records.into();
}

fn is_valid_frametime_ms(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

fn percentile_f64(mut values: Vec<f64>, percentile: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }

    values.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));

    let rank = ((values.len() as f64 * percentile).ceil() as usize).saturating_sub(1);
    values[rank.min(values.len() - 1)]
}

#[cfg(test)]
mod tests;
