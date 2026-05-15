use std::{
    collections::{BTreeSet, VecDeque},
    time::Duration,
};

use crate::{
    autotune::{
        objective::ObjectiveSignals,
        quality::{OnlineDataQuality, OnlineDataQualityInput, OnlineDataQualityPolicy},
    },
    diagnosis::LiveDiagnosisEntry,
    recorder::{
        BlockIoRecord, CpuFreqRecord, ForegroundEvent, FrameEvent, GpuSample, IntervalRecord,
        IrqEventRecord,
    },
};

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct WindowScore {
    pub duration_ms: u64,
    pub interval_count: usize,
    pub scored_task_count: usize,
    pub scored_samples: u64,
    pub score_total: u64,
    pub over_1ms: u64,
    pub over_2ms: u64,
    pub over_5ms: u64,
    pub max_latency_ns: u64,
    pub frame_count: usize,
    pub frame_p99_ms: f64,
    pub frame_max_ms: f64,
    pub data_quality: OnlineDataQuality,
}

const GPU_THERMAL_DEGRADED_MILLIDEGREES: u32 = 85_000;
const GPU_POWER_LIMIT_BUSY_PERCENT: u32 = 95;
const GPU_POWER_LIMIT_LOW_CLOCK_MHZ: u32 = 300;

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct RollingWindow {
    pub duration: Duration,
    pub intervals: VecDeque<IntervalRecord>,
    pub frames: VecDeque<FrameEvent>,
    pub diagnoses: VecDeque<LiveDiagnosisEntry>,
    pub irq_events: VecDeque<IrqEventRecord>,
    pub block_io_events: VecDeque<BlockIoRecord>,
    pub gpu_samples: VecDeque<GpuSample>,
    pub cpu_freq_events: VecDeque<CpuFreqRecord>,
    pub foreground_events: VecDeque<ForegroundEvent>,
}

#[allow(dead_code)]
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
        }
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
        self.intervals.push_back(record);
        self.prune_to(elapsed_ms);
    }

    pub fn push_intervals<I>(&mut self, records: I)
    where
        I: IntoIterator<Item = IntervalRecord>,
    {
        let mut latest_elapsed_ms = None;

        for record in records {
            latest_elapsed_ms = Some(latest_elapsed_ms.unwrap_or(0).max(record.elapsed_ms));
            self.intervals.push_back(record);
        }

        if let Some(elapsed_ms) = latest_elapsed_ms {
            self.prune_to(elapsed_ms);
        }
    }

    pub fn push_frame(&mut self, frame: FrameEvent) {
        let elapsed_ms = frame.elapsed_ms;
        self.frames.push_back(frame);
        self.prune_to(elapsed_ms);
    }

    pub fn push_irq_event(&mut self, event: IrqEventRecord) {
        let elapsed_ms = event.elapsed_ms;
        self.irq_events.push_back(event);
        if let Some(elapsed_ms) = elapsed_ms {
            self.prune_to(elapsed_ms);
        }
    }

    pub fn push_block_io_event(&mut self, event: BlockIoRecord) {
        let elapsed_ms = event.elapsed_ms;
        self.block_io_events.push_back(event);
        self.prune_to(elapsed_ms);
    }

    pub fn push_gpu_sample(&mut self, sample: GpuSample) {
        let elapsed_ms = sample.elapsed_ms;
        self.gpu_samples.push_back(sample);
        self.prune_to(elapsed_ms);
    }

    pub fn push_cpu_freq_event(&mut self, event: CpuFreqRecord) {
        let elapsed_ms = event.elapsed_ms;
        self.cpu_freq_events.push_back(event);
        self.prune_to(elapsed_ms);
    }

    pub fn push_foreground_event(&mut self, event: ForegroundEvent) {
        let elapsed_ms = event.elapsed_ms;
        self.foreground_events.push_back(event);
        self.prune_to(elapsed_ms);
    }

    pub fn push_diagnosis(&mut self, diagnosis: LiveDiagnosisEntry) {
        let elapsed_ms = diagnosis.elapsed_ms;
        self.diagnoses.push_back(diagnosis);
        self.prune_to(elapsed_ms);
    }

    pub fn prune_to(&mut self, now_elapsed_ms: u64) {
        let start_ms = self.window_start_ms_for(now_elapsed_ms);

        prune_front_by_elapsed(&mut self.intervals, start_ms, |record| record.elapsed_ms);
        prune_front_by_elapsed(&mut self.frames, start_ms, |frame| frame.elapsed_ms);
        prune_front_by_elapsed(&mut self.diagnoses, start_ms, |diagnosis| {
            diagnosis.elapsed_ms
        });
        prune_front_by_elapsed(&mut self.irq_events, start_ms, |event| {
            event.elapsed_ms.unwrap_or(0)
        });
        prune_front_by_elapsed(&mut self.block_io_events, start_ms, |event| {
            event.elapsed_ms
        });
        prune_front_by_elapsed(&mut self.gpu_samples, start_ms, |sample| sample.elapsed_ms);
        prune_front_by_elapsed(&mut self.cpu_freq_events, start_ms, |event| {
            event.elapsed_ms
        });
        prune_front_by_elapsed(&mut self.foreground_events, start_ms, |event| {
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
                .filter(|value| value.is_finite() && *value >= 0.0)
                .collect(),
            0.99,
        )
    }

    pub fn frame_max_ms(&self) -> f64 {
        self.frames
            .iter()
            .map(|frame| frame.frametime_ms)
            .filter(|value| value.is_finite() && *value >= 0.0)
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
        let gpu_power_limited =
            (!self.gpu_samples.is_empty()).then_some(self.gpu_samples.iter().any(|sample| {
                sample.gpu_busy_percent.unwrap_or(0) >= GPU_POWER_LIMIT_BUSY_PERCENT
                    && sample.gpu_clock_mhz.unwrap_or(u32::MAX) <= GPU_POWER_LIMIT_LOW_CLOCK_MHZ
            }));

        let has_block_io_events = !self.block_io_events.is_empty();
        let has_irq_events = !self.irq_events.is_empty();
        let has_dirty_writeback = dirty_writeback_events > 0;

        ObjectiveSignals {
            block_io_overlap_count: has_block_io_events.then_some(block_io_overlap_count),
            block_io_worst_latency_ns: has_block_io_events.then_some(block_io_worst_latency_ns),
            irq_overlap_count: has_irq_events.then_some(irq_overlap_count),
            irq_worst_overlap_ns: has_irq_events.then_some(irq_worst_overlap_ns),
            irq_hot_irq: irq_worst_event.map(|event| event.irq),
            irq_hot_cpu: irq_worst_event.map(|event| event.cpu),
            thermal_degraded,
            thermal_throttle_count: thermal_degraded.map(|_| thermal_throttle_count),
            cpu_power_limited,
            cpu_power_limited_cpu: cpu_power_limited_event.map(|event| event.cpu),
            gpu_power_limited,
            gpu_busy_percent: latest_gpu.and_then(|sample| sample.gpu_busy_percent),
            gpu_clock_mhz: latest_gpu.and_then(|sample| sample.gpu_clock_mhz),
            gpu_temp_millidegrees: latest_gpu.and_then(|sample| sample.temp_millidegrees),
            gpu_active_render_node: None,
            memory_pressure_some_avg10_percent: None,
            swap_activity_events: None,
            dirty_writeback_events: has_dirty_writeback.then_some(dirty_writeback_events),
            frame_p99_ms: Some(self.frame_p99_ms()),
            foreground_over_5ms: Some(
                self.intervals
                    .iter()
                    .map(|record| record.over_5ms)
                    .fold(0_u64, u64::saturating_add),
            ),
        }
    }

    pub fn score(&self) -> WindowScore {
        self.score_with_quality_policy(&OnlineDataQualityPolicy::default())
    }

    pub(crate) fn score_with_quality_policy(
        &self,
        quality_policy: &OnlineDataQualityPolicy,
    ) -> WindowScore {
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
        let score_total = over_5ms
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

        WindowScore {
            duration_ms: self.duration_ms(),
            interval_count,
            scored_task_count,
            scored_samples,
            score_total,
            over_1ms,
            over_2ms,
            over_5ms,
            max_latency_ns,
            frame_count,
            frame_p99_ms,
            frame_max_ms,
            data_quality,
        }
    }
}

impl Default for RollingWindow {
    fn default() -> Self {
        Self::new(Duration::from_secs(30))
    }
}

#[allow(dead_code)]
fn prune_front_by_elapsed<T, F>(items: &mut VecDeque<T>, start_ms: u64, elapsed_ms: F)
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

#[allow(dead_code)]
fn percentile_f64(mut values: Vec<f64>, percentile: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }

    values.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));

    let rank = ((values.len() as f64 * percentile).ceil() as usize).saturating_sub(1);
    values[rank.min(values.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        diagnosis::{Confidence, LiveDiagnosisEntry, StutterCause},
        process_tree::TaskClass,
    };

    fn interval(elapsed_ms: u64, samples: u64) -> IntervalRecord {
        IntervalRecord {
            elapsed_ms,
            samples,
            ..Default::default()
        }
    }

    fn frame(elapsed_ms: u64, frametime_ms: f64) -> FrameEvent {
        FrameEvent {
            elapsed_ms,
            frametime_ms,
        }
    }

    fn irq_event(elapsed_ms: u64, duration_ns: u64) -> IrqEventRecord {
        IrqEventRecord {
            elapsed_ms: Some(elapsed_ms),
            irq: 44,
            cpu: 2,
            enter_ns: 1_000,
            exit_ns: 1_000 + duration_ns,
            duration_ns,
        }
    }

    fn block_io_event(elapsed_ms: u64, duration_ns: u64) -> BlockIoRecord {
        BlockIoRecord {
            elapsed_ms,
            tid: 77,
            dev: 1,
            nr_sector: 8,
            sector: 99,
            duration_ns,
            timestamp_ns: 2_000 + duration_ns,
            rwbs: "R".to_owned(),
            ..BlockIoRecord::default()
        }
    }

    fn gpu_sample(elapsed_ms: u64, temp_millidegrees: u32) -> GpuSample {
        GpuSample {
            elapsed_ms,
            temp_millidegrees: Some(temp_millidegrees),
            gpu_busy_percent: Some(96),
            gpu_clock_mhz: Some(250),
            ..GpuSample::default()
        }
    }

    fn diagnosis(elapsed_ms: u64, cause: StutterCause) -> LiveDiagnosisEntry {
        LiveDiagnosisEntry {
            elapsed_ms,
            cause,
            confidence: Confidence::Medium,
            anchor_class: TaskClass::Game,
            anchor_comm: "RenderThread".to_owned(),
            evidence: vec!["test evidence".to_owned()],
        }
    }

    #[test]
    fn default_window_is_thirty_seconds() {
        let window = RollingWindow::default();

        assert_eq!(window.duration, Duration::from_secs(30));
        assert!(window.is_empty());
    }

    #[test]
    fn push_interval_prunes_old_intervals_by_duration() {
        let mut window = RollingWindow::new(Duration::from_secs(2));

        window.push_interval(interval(1000, 10));
        window.push_interval(interval(2500, 20));
        window.push_interval(interval(3501, 30));

        assert_eq!(
            window
                .intervals
                .iter()
                .map(|record| record.elapsed_ms)
                .collect::<Vec<_>>(),
            vec![2500, 3501]
        );
        assert_eq!(window.scored_samples(), 50);
    }

    #[test]
    fn push_frame_prunes_old_frames_by_duration() {
        let mut window = RollingWindow::new(Duration::from_secs(1));

        window.push_frame(frame(1000, 16.0));
        window.push_frame(frame(1500, 17.0));
        window.push_frame(frame(2101, 18.0));

        assert_eq!(
            window
                .frames
                .iter()
                .map(|frame| frame.elapsed_ms)
                .collect::<Vec<_>>(),
            vec![1500, 2101]
        );
    }

    #[test]
    fn push_diagnosis_prunes_old_diagnoses_by_duration() {
        let mut window = RollingWindow::new(Duration::from_secs(3));

        window.push_diagnosis(diagnosis(1000, StutterCause::Unknown));
        window.push_diagnosis(diagnosis(3000, StutterCause::GpuBoundCandidate));
        window.push_diagnosis(diagnosis(4501, StutterCause::GameThreadSchedulerDelay));

        assert_eq!(
            window
                .diagnoses
                .iter()
                .map(|diagnosis| diagnosis.elapsed_ms)
                .collect::<Vec<_>>(),
            vec![3000, 4501]
        );
    }

    #[test]
    fn prune_to_prunes_all_streams_using_same_cutoff() {
        let mut window = RollingWindow::new(Duration::from_secs(2));
        window.intervals.push_back(interval(1000, 10));
        window.intervals.push_back(interval(3000, 20));
        window.frames.push_back(frame(999, 16.0));
        window.frames.push_back(frame(2500, 22.0));
        window
            .diagnoses
            .push_back(diagnosis(1500, StutterCause::Unknown));
        window
            .diagnoses
            .push_back(diagnosis(3200, StutterCause::CpuPressureCandidate));

        window.prune_to(3500);

        assert_eq!(
            window
                .intervals
                .iter()
                .map(|record| record.elapsed_ms)
                .collect::<Vec<_>>(),
            vec![3000]
        );
        assert_eq!(
            window
                .frames
                .iter()
                .map(|frame| frame.elapsed_ms)
                .collect::<Vec<_>>(),
            vec![2500]
        );
        assert_eq!(
            window
                .diagnoses
                .iter()
                .map(|diagnosis| diagnosis.elapsed_ms)
                .collect::<Vec<_>>(),
            vec![1500, 3200]
        );
    }

    #[test]
    fn retain_latest_window_uses_latest_event_across_streams() {
        let mut window = RollingWindow::new(Duration::from_secs(1));
        window.intervals.push_back(interval(1000, 10));
        window.frames.push_back(frame(1500, 17.0));
        window
            .diagnoses
            .push_back(diagnosis(2300, StutterCause::CpuPressureCandidate));

        window.retain_latest_window();

        assert!(window.intervals.is_empty());
        assert_eq!(
            window
                .frames
                .iter()
                .map(|frame| frame.elapsed_ms)
                .collect::<Vec<_>>(),
            vec![1500]
        );
        assert_eq!(
            window
                .diagnoses
                .iter()
                .map(|diagnosis| diagnosis.elapsed_ms)
                .collect::<Vec<_>>(),
            vec![2300]
        );
    }

    #[test]
    fn push_intervals_prunes_once_using_latest_inserted_elapsed_ms() {
        let mut window = RollingWindow::new(Duration::from_secs(2));

        window.push_intervals(vec![
            interval(1000, 1),
            interval(2000, 2),
            interval(3501, 3),
        ]);

        assert_eq!(
            window
                .intervals
                .iter()
                .map(|record| record.elapsed_ms)
                .collect::<Vec<_>>(),
            vec![2000, 3501]
        );
        assert_eq!(window.scored_samples(), 5);
    }

    #[test]
    fn latest_elapsed_ms_uses_max_across_streams() {
        let mut window = RollingWindow::new(Duration::from_secs(10));
        window.intervals.push_back(interval(1000, 10));
        window.frames.push_back(frame(4000, 16.0));
        window
            .diagnoses
            .push_back(diagnosis(2500, StutterCause::Unknown));

        assert_eq!(window.latest_elapsed_ms(), Some(4000));
    }

    #[test]
    fn frame_stats_ignore_non_finite_and_negative_values() {
        let mut window = RollingWindow::new(Duration::from_secs(10));
        window.push_frame(frame(1000, 16.0));
        window.push_frame(frame(1100, f64::NAN));
        window.push_frame(frame(1200, -1.0));
        window.push_frame(frame(1300, 33.0));

        assert_eq!(window.frame_max_ms(), 33.0);
        assert_eq!(window.frame_p99_ms(), 33.0);
    }

    #[test]
    fn window_score_aggregates_latency_frames_and_samples() {
        let mut window = RollingWindow::new(Duration::from_secs(5));
        window.push_interval(IntervalRecord {
            elapsed_ms: 1000,
            task: 42,
            samples: 30,
            over_1ms: 3,
            over_2ms: 2,
            over_5ms: 1,
            max_ns: 6_000_000,
            ..Default::default()
        });
        window.push_interval(IntervalRecord {
            elapsed_ms: 2000,
            task: 43,
            samples: 70,
            over_1ms: 4,
            over_2ms: 1,
            over_5ms: 0,
            max_ns: 4_000_000,
            ..Default::default()
        });
        window.push_frame(frame(1500, 16.0));
        window.push_frame(frame(1600, 33.0));

        let score = window.score();

        assert_eq!(score.duration_ms, 5000);
        assert_eq!(score.interval_count, 2);
        assert_eq!(score.scored_samples, 100);
        assert_eq!(score.over_1ms, 7);
        assert_eq!(score.over_2ms, 3);
        assert_eq!(score.over_5ms, 1);
        assert_eq!(score.score_total, 167);
        assert_eq!(score.max_latency_ns, 6_000_000);
        assert_eq!(score.frame_count, 2);
        assert_eq!(score.frame_p99_ms, 33.0);
        assert_eq!(score.frame_max_ms, 33.0);
    }

    #[test]
    fn window_score_quality_is_high_when_online_quality_gates_pass() {
        let mut window = RollingWindow::new(Duration::from_secs(10));

        for elapsed_ms in [1000, 2000, 3000, 4000, 5000] {
            window.push_interval(IntervalRecord {
                elapsed_ms,
                task: 42,
                samples: 20,
                over_1ms: 1,
                max_ns: 2_000_000,
                ..Default::default()
            });
        }

        let score = window.score();

        assert_eq!(score.interval_count, 5);
        assert_eq!(score.scored_samples, 100);
        assert_eq!(score.data_quality, OnlineDataQuality::High);
    }

    #[test]
    fn window_score_default_quality_policy_does_not_require_frames() {
        let mut window = RollingWindow::new(Duration::from_secs(10));

        for elapsed_ms in [1000, 2000, 3000, 4000, 5000] {
            window.push_interval(IntervalRecord {
                elapsed_ms,
                task: 42,
                samples: 20,
                over_1ms: 1,
                max_ns: 2_000_000,
                ..Default::default()
            });
        }

        let score = window.score();

        assert_eq!(score.frame_count, 0);
        assert_eq!(score.data_quality, OnlineDataQuality::High);
    }

    #[test]
    fn window_score_quality_is_low_when_policy_requires_frames_and_none_exist() {
        let mut window = RollingWindow::new(Duration::from_secs(10));

        for elapsed_ms in [1000, 2000, 3000, 4000, 5000] {
            window.push_interval(IntervalRecord {
                elapsed_ms,
                task: 42,
                samples: 20,
                over_1ms: 1,
                max_ns: 2_000_000,
                ..Default::default()
            });
        }

        let quality_policy = OnlineDataQualityPolicy {
            frame_data_policy: crate::autotune::quality::FrameDataPolicy::Required,
            ..OnlineDataQualityPolicy::default()
        };
        let score = window.score_with_quality_policy(&quality_policy);

        assert_eq!(score.frame_count, 0);
        assert!(score.data_quality.is_low());
        assert!(
            score
                .data_quality
                .reasons()
                .iter()
                .any(|reason| reason.contains("no frame data"))
        );
    }

    #[test]
    fn window_score_quality_is_low_for_empty_window() {
        let window = RollingWindow::new(Duration::from_secs(10));

        let score = window.score();

        assert_eq!(score.interval_count, 0);
        assert_eq!(score.scored_samples, 0);
        assert_eq!(score.score_total, 0);
        assert!(score.data_quality.is_low());
        assert!(
            score
                .data_quality
                .reasons()
                .iter()
                .any(|reason| reason.contains("fewer than min_scored_intervals"))
        );
    }

    #[test]
    fn window_score_quality_is_low_when_drop_counters_are_nonzero() {
        let mut window = RollingWindow::new(Duration::from_secs(10));

        for elapsed_ms in [1000, 2000, 3000, 4000, 5000] {
            window.push_interval(IntervalRecord {
                elapsed_ms,
                task: 42,
                samples: 20,
                drop_counters: crate::ebpf_loader::DropCountersSnapshot {
                    ringbuf_reserve_failed: if elapsed_ms == 5000 { 1 } else { 0 },
                    ..Default::default()
                },
                ..Default::default()
            });
        }

        let score = window.score();

        assert!(score.data_quality.is_low());
        assert!(
            score
                .data_quality
                .reasons()
                .iter()
                .any(|reason| reason.contains("drop counters above policy max"))
        );
    }

    #[test]
    fn recent_diagnoses_vec_returns_cloned_diagnoses_in_order() {
        let mut window = RollingWindow::new(Duration::from_secs(10));
        window.push_diagnosis(diagnosis(1000, StutterCause::Unknown));
        window.push_diagnosis(diagnosis(2000, StutterCause::GpuBoundCandidate));

        let diagnoses = window.recent_diagnoses_vec();

        assert_eq!(diagnoses.len(), 2);
        assert_eq!(diagnoses[0].elapsed_ms, 1000);
        assert_eq!(diagnoses[1].elapsed_ms, 2000);
    }

    #[test]
    fn clear_removes_all_streams() {
        let mut window = RollingWindow::new(Duration::from_secs(10));
        window.push_interval(interval(1000, 10));
        window.push_frame(frame(1000, 16.0));
        window.push_diagnosis(diagnosis(1000, StutterCause::Unknown));

        window.clear();

        assert!(window.is_empty());
        assert_eq!(window.total_event_count(), 0);
    }

    #[test]
    fn objective_signals_mark_missing_io_and_irq_evidence_as_none() {
        let window = RollingWindow::new(Duration::from_secs(30));

        let signals = window.objective_signals();

        assert_eq!(signals.block_io_overlap_count, None);
        assert_eq!(signals.block_io_worst_latency_ns, None);
        assert_eq!(signals.irq_overlap_count, None);
        assert_eq!(signals.irq_worst_overlap_ns, None);
        assert_eq!(signals.irq_hot_irq, None);
        assert_eq!(signals.irq_hot_cpu, None);
        assert_eq!(signals.cpu_power_limited_cpu, None);
        assert_eq!(signals.gpu_busy_percent, None);
        assert_eq!(signals.gpu_clock_mhz, None);
        assert_eq!(signals.gpu_temp_millidegrees, None);
        assert_eq!(signals.dirty_writeback_events, None);
    }

    #[test]
    fn objective_signals_collect_io_irq_thermal_and_power_indicators() {
        let mut window = RollingWindow::new(Duration::from_secs(30));
        window.push_interval(interval(1_000, 10));
        window.push_irq_event(irq_event(1_100, 3_000_000));
        window.push_block_io_event(block_io_event(1_200, 8_000_000));
        window.push_gpu_sample(gpu_sample(1_300, 90_000));
        window.push_cpu_freq_event(CpuFreqRecord {
            elapsed_ms: 1_400,
            cpu: 0,
            freq_khz: 0,
            timestamp_ns: 123,
        });

        let signals = window.objective_signals();

        assert_eq!(signals.block_io_overlap_count, Some(1));
        assert_eq!(signals.block_io_worst_latency_ns, Some(8_000_000));
        assert_eq!(signals.irq_overlap_count, Some(1));
        assert_eq!(signals.irq_worst_overlap_ns, Some(3_000_000));
        assert_eq!(signals.irq_hot_irq, Some(44));
        assert_eq!(signals.irq_hot_cpu, Some(2));
        assert_eq!(signals.thermal_degraded, Some(true));
        assert_eq!(signals.thermal_throttle_count, Some(1));
        assert_eq!(signals.cpu_power_limited, Some(true));
        assert_eq!(signals.cpu_power_limited_cpu, Some(0));
        assert_eq!(signals.gpu_power_limited, Some(true));
        assert_eq!(signals.gpu_busy_percent, Some(96));
        assert_eq!(signals.gpu_clock_mhz, Some(250));
        assert_eq!(signals.gpu_temp_millidegrees, Some(90_000));
    }
}
