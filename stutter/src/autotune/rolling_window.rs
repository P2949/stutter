use std::{collections::VecDeque, time::Duration};

use crate::{
    diagnosis::LiveDiagnosisEntry,
    recorder::{FrameEvent, IntervalRecord},
};

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct RollingWindow {
    pub duration: Duration,
    pub intervals: VecDeque<IntervalRecord>,
    pub frames: VecDeque<FrameEvent>,
    pub diagnoses: VecDeque<LiveDiagnosisEntry>,
}

#[allow(dead_code)]
impl RollingWindow {
    pub fn new(duration: Duration) -> Self {
        Self {
            duration,
            intervals: VecDeque::new(),
            frames: VecDeque::new(),
            diagnoses: VecDeque::new(),
        }
    }

    pub fn duration_ms(&self) -> u64 {
        self.duration.as_millis().min(u128::from(u64::MAX)) as u64
    }

    pub fn is_empty(&self) -> bool {
        self.intervals.is_empty() && self.frames.is_empty() && self.diagnoses.is_empty()
    }

    pub fn clear(&mut self) {
        self.intervals.clear();
        self.frames.clear();
        self.diagnoses.clear();
    }

    pub fn latest_elapsed_ms(&self) -> Option<u64> {
        [
            self.intervals.back().map(|record| record.elapsed_ms),
            self.frames.back().map(|frame| frame.elapsed_ms),
            self.diagnoses.back().map(|diagnosis| diagnosis.elapsed_ms),
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
}
