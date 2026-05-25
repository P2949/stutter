use std::collections::{BTreeMap, BTreeSet};

use super::experiment::WindowScore;
use crate::{
    ebpf_loader::DropCountersSnapshot,
    metrics::IntervalRecord,
    process_tree::TaskClass,
    scorer::{class_contributes_to_score, score_from_interval_records},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BaselineWindowConfig {
    pub baseline_window_seconds: u64,
    pub min_scored_intervals: usize,
    pub min_scored_samples: u64,
}

impl Default for BaselineWindowConfig {
    fn default() -> Self {
        Self {
            baseline_window_seconds: 30,
            min_scored_intervals: 10,
            min_scored_samples: 100,
        }
    }
}

impl BaselineWindowConfig {
    pub fn baseline_window_ms(&self) -> u64 {
        self.baseline_window_seconds.saturating_mul(1_000)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TargetIdentitySnapshot {
    pub target_present: bool,
    pub target_root_pid: Option<u32>,
    pub active_target_count: usize,
    pub identities: BTreeSet<TaskIdentityKey>,
}

impl TargetIdentitySnapshot {
    pub fn absent() -> Self {
        Self::default()
    }

    pub fn from_interval_records(target_root_pid: Option<u32>, records: &[IntervalRecord]) -> Self {
        let identities = records
            .iter()
            .filter(|record| record.active)
            .map(TaskIdentityKey::from_interval_record)
            .collect::<BTreeSet<_>>();

        Self {
            target_present: !identities.is_empty(),
            target_root_pid,
            active_target_count: identities.len(),
            identities,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TaskIdentityKey {
    pub task: u32,
    pub process_pid: Option<u32>,
    pub class: TaskClass,
    pub comm: String,
    pub process_comm: String,
}

impl TaskIdentityKey {
    pub fn from_interval_record(record: &IntervalRecord) -> Self {
        Self {
            task: record.task,
            process_pid: record.process_pid,
            class: record.class,
            comm: record.comm.clone(),
            process_comm: record.process_comm.clone(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct BaselineWindowState {
    config: BaselineWindowConfig,
    started_unix_nanos: Option<u128>,
    first_elapsed_ms: Option<u64>,
    last_elapsed_ms: Option<u64>,
    records: Vec<IntervalRecord>,
    scored_intervals: usize,
    scored_samples: u64,
    scored_tasks: BTreeSet<u32>,
    target_identity: Option<TargetIdentitySnapshot>,
    target_disappeared: bool,
    target_identity_shifted: bool,
    drop_counter_total: u64,
}

impl BaselineWindowState {
    pub fn new(config: BaselineWindowConfig) -> Self {
        Self {
            config,
            ..Self::default()
        }
    }

    pub fn config(&self) -> &BaselineWindowConfig {
        &self.config
    }

    pub fn observe_interval(
        &mut self,
        now_unix_nanos: u128,
        elapsed_ms: u64,
        records: &[IntervalRecord],
        drop_counters: &DropCountersSnapshot,
        target_identity: TargetIdentitySnapshot,
    ) -> BaselineWindowStatus {
        if self.started_unix_nanos.is_none() {
            self.started_unix_nanos = Some(now_unix_nanos);
            self.first_elapsed_ms = Some(elapsed_ms);
        }
        self.last_elapsed_ms = Some(elapsed_ms);

        self.drop_counter_total = self
            .drop_counter_total
            .saturating_add(drop_counters.total());

        if !target_identity.target_present {
            self.target_disappeared = true;
        }

        match &self.target_identity {
            None => {
                self.target_identity = Some(target_identity);
            }
            Some(previous) => {
                if target_identity.target_present && previous != &target_identity {
                    self.target_identity_shifted = true;
                }
            }
        }

        let scored_this_interval = records
            .iter()
            .filter(|record| class_contributes_to_score(record.class))
            .collect::<Vec<_>>();

        let interval_samples = scored_this_interval
            .iter()
            .fold(0u64, |sum, record| sum.saturating_add(record.samples));

        if interval_samples > 0 {
            self.scored_intervals = self.scored_intervals.saturating_add(1);
            self.scored_samples = self.scored_samples.saturating_add(interval_samples);
            for record in scored_this_interval {
                self.scored_tasks.insert(record.task);
            }
        }

        self.records.extend(records.iter().cloned());

        self.status(now_unix_nanos)
    }

    pub fn status(&self, now_unix_nanos: u128) -> BaselineWindowStatus {
        let elapsed_window_ms = self.elapsed_window_ms(now_unix_nanos);
        let mut reasons = Vec::new();

        if elapsed_window_ms < self.config.baseline_window_ms() {
            reasons.push(format!(
                "baseline window not complete: elapsed_ms={} required_ms={}",
                elapsed_window_ms,
                self.config.baseline_window_ms()
            ));
        }

        if self.scored_intervals < self.config.min_scored_intervals {
            reasons.push(format!(
                "fewer than min_scored_intervals: scored_intervals={} min_scored_intervals={}",
                self.scored_intervals, self.config.min_scored_intervals
            ));
        }

        if self.scored_samples < self.config.min_scored_samples {
            reasons.push(format!(
                "fewer than min_scored_samples: scored_samples={} min_scored_samples={}",
                self.scored_samples, self.config.min_scored_samples
            ));
        }

        if self.scored_tasks.is_empty() {
            reasons.push("zero scored tasks".to_owned());
        }

        if self.target_identity.is_none() {
            reasons.push("target identity has not been observed".to_owned());
        }

        if self.target_disappeared {
            reasons.push("target disappeared during baseline window".to_owned());
        }

        if self.target_identity_shifted {
            reasons.push("target identity shifted during baseline window".to_owned());
        }

        if self.drop_counter_total > 0 {
            reasons.push(format!(
                "drop counters observed during baseline window: drop_counter_total={}",
                self.drop_counter_total
            ));
        }

        if reasons.is_empty() {
            BaselineWindowStatus::Ready {
                score: self.to_window_score(now_unix_nanos),
            }
        } else {
            BaselineWindowStatus::Collecting {
                elapsed_ms: elapsed_window_ms,
                scored_intervals: self.scored_intervals,
                scored_samples: self.scored_samples,
                scored_task_count: self.scored_tasks.len(),
                drop_counter_total: self.drop_counter_total,
                reasons,
            }
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new(self.config.clone());
    }

    fn elapsed_window_ms(&self, now_unix_nanos: u128) -> u64 {
        match self.started_unix_nanos {
            Some(started_unix_nanos) => now_unix_nanos
                .saturating_sub(started_unix_nanos)
                .checked_div(1_000_000)
                .unwrap_or(0)
                .min(u64::MAX as u128) as u64,
            None => 0,
        }
    }

    fn to_window_score(&self, now_unix_nanos: u128) -> WindowScore {
        WindowScore {
            started_unix_nanos: self.started_unix_nanos.unwrap_or(now_unix_nanos),
            finished_unix_nanos: now_unix_nanos,
            interval_count: self.scored_intervals,
            scored_samples: self.scored_samples,
            scored_task_count: self.scored_tasks.len(),
            score: score_from_interval_records(&self.records),
        }
    }
}

#[derive(Clone, Debug)]
pub enum BaselineWindowStatus {
    Collecting {
        elapsed_ms: u64,
        scored_intervals: usize,
        scored_samples: u64,
        scored_task_count: usize,
        drop_counter_total: u64,
        reasons: Vec<String>,
    },
    Ready {
        score: WindowScore,
    },
}

impl BaselineWindowStatus {
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }

    pub fn reasons(&self) -> &[String] {
        match self {
            Self::Collecting { reasons, .. } => reasons,
            Self::Ready { .. } => &[],
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct BaselineWindowAccumulator {
    by_elapsed_ms: BTreeMap<u64, Vec<IntervalRecord>>,
}

impl BaselineWindowAccumulator {
    pub fn push_interval_records(&mut self, records: &[IntervalRecord]) {
        for record in records {
            self.by_elapsed_ms
                .entry(record.elapsed_ms)
                .or_default()
                .push(record.clone());
        }
    }

    pub fn ordered_records(&self) -> Vec<IntervalRecord> {
        self.by_elapsed_ms
            .values()
            .flat_map(|records| records.iter().cloned())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::IntervalRecord;

    fn record(elapsed_ms: u64, task: u32, samples: u64, class: TaskClass) -> IntervalRecord {
        IntervalRecord {
            elapsed_ms,
            task,
            active: true,
            class,
            comm: format!("task-{task}"),
            process_pid: Some(42),
            process_comm: "Game.exe".into(),
            samples,
            stored_samples: samples,
            truncated_samples: 0,
            min_ns: 0,
            avg_ns: 0,
            p95_ns: 0,
            p99_ns: 0,
            max_ns: 7_000_000,
            over_1ms: 3,
            over_2ms: 2,
            over_5ms: 1,
            busiest_cpu: None,
            busiest_cpu_samples: 0,
            worst_cpu: None,
            worst_cpu_max_ns: 0,
            spikiest_cpu: None,
            spikiest_cpu_spikes: 0,
            major_faults: 0,
            minor_faults: 0,
            cpu_psi_some: 0.0,
            mem_psi_some: 0.0,
            mem_psi_full: 0.0,
            mem_psi_delta_us: 0,
            mem_psi_spike: false,
            io_psi_some: 0.0,
            io_psi_full: 0.0,
            percentile_scope: "all".to_owned(),
            histogram: Vec::new(),
            drop_counters: DropCountersSnapshot::default(),
            cpu_perf: None,
        }
    }

    fn identity(records: &[IntervalRecord]) -> TargetIdentitySnapshot {
        TargetIdentitySnapshot::from_interval_records(Some(42), records)
    }

    fn config() -> BaselineWindowConfig {
        BaselineWindowConfig {
            baseline_window_seconds: 30,
            min_scored_intervals: 10,
            min_scored_samples: 100,
        }
    }

    #[test]
    fn baseline_window_waits_for_duration_min_intervals_and_min_samples() {
        let mut state = BaselineWindowState::new(config());

        for idx in 0..9 {
            let elapsed_ms = (idx + 1) * 1_000;
            let records = vec![record(elapsed_ms, 7, 10, TaskClass::Game)];
            let status = state.observe_interval(
                (elapsed_ms as u128) * 1_000_000,
                elapsed_ms,
                &records,
                &DropCountersSnapshot::default(),
                identity(&records),
            );
            assert!(!status.is_ready());
        }

        let records = vec![record(30_000, 7, 10, TaskClass::Game)];
        let status = state.observe_interval(
            31_000_000_000,
            30_000,
            &records,
            &DropCountersSnapshot::default(),
            identity(&records),
        );

        match status {
            BaselineWindowStatus::Ready { score } => {
                assert_eq!(score.interval_count, 10);
                assert_eq!(score.scored_samples, 100);
                assert_eq!(score.scored_task_count, 1);
                assert_eq!(score.score.total, 1_430);
            }
            other => panic!("expected ready baseline window, got {other:?}"),
        }
    }

    #[test]
    fn baseline_window_rejects_too_few_intervals() {
        let mut state = BaselineWindowState::new(config());

        for idx in 0..5 {
            let elapsed_ms = (idx + 1) * 6_000;
            let records = vec![record(elapsed_ms, 7, 20, TaskClass::Game)];
            state.observe_interval(
                (elapsed_ms as u128) * 1_000_000,
                elapsed_ms,
                &records,
                &DropCountersSnapshot::default(),
                identity(&records),
            );
        }

        let status = state.status(30_000_000_000);

        assert!(!status.is_ready());
        assert!(
            status
                .reasons()
                .iter()
                .any(|reason| reason.contains("fewer than min_scored_intervals"))
        );
    }

    #[test]
    fn baseline_window_rejects_too_few_samples() {
        let mut state = BaselineWindowState::new(config());

        for idx in 0..10 {
            let elapsed_ms = (idx + 1) * 3_000;
            let records = vec![record(elapsed_ms, 7, 5, TaskClass::Game)];
            state.observe_interval(
                (elapsed_ms as u128) * 1_000_000,
                elapsed_ms,
                &records,
                &DropCountersSnapshot::default(),
                identity(&records),
            );
        }

        let status = state.status(30_000_000_000);

        assert!(!status.is_ready());
        assert!(
            status
                .reasons()
                .iter()
                .any(|reason| reason.contains("fewer than min_scored_samples"))
        );
    }

    #[test]
    fn baseline_window_requires_target_present_for_whole_window() {
        let mut state = BaselineWindowState::new(config());

        for idx in 0..10 {
            let elapsed_ms = (idx + 1) * 3_000;
            let records = vec![record(elapsed_ms, 7, 10, TaskClass::Game)];
            let target_identity = if idx == 3 {
                TargetIdentitySnapshot::absent()
            } else {
                identity(&records)
            };
            state.observe_interval(
                (elapsed_ms as u128) * 1_000_000,
                elapsed_ms,
                &records,
                &DropCountersSnapshot::default(),
                target_identity,
            );
        }

        let status = state.status(30_000_000_000);

        assert!(!status.is_ready());
        assert!(
            status
                .reasons()
                .iter()
                .any(|reason| reason == "target disappeared during baseline window")
        );
    }

    #[test]
    fn baseline_window_requires_zero_drop_counters() {
        let mut state = BaselineWindowState::new(config());
        let drop_counters = DropCountersSnapshot {
            wakeup_data_insert_failed: 1,
            wakeup_data_stale_entries: 0,
            wakeup_data_replaced_entries: 0,
            wakeup_data_consumed_read_failed: 0,
            ringbuf_reserve_failed: 0,
            irq_start_times_insert_failed: 0,
            block_start_insert_failed: 0,
            block_fallback_key_collisions: 0,
            cpu_accounting_untracked: 0,
            block_zero_keys: 0,
            drm_fence_missing_start: 0,
        };

        for idx in 0..10 {
            let elapsed_ms = (idx + 1) * 3_000;
            let records = vec![record(elapsed_ms, 7, 10, TaskClass::Game)];
            let counters = if idx == 4 {
                &drop_counters
            } else {
                &DropCountersSnapshot::default()
            };
            state.observe_interval(
                (elapsed_ms as u128) * 1_000_000,
                elapsed_ms,
                &records,
                counters,
                identity(&records),
            );
        }

        let status = state.status(30_000_000_000);

        assert!(!status.is_ready());
        assert!(
            status
                .reasons()
                .iter()
                .any(|reason| reason.contains("drop counters observed"))
        );
    }

    #[test]
    fn baseline_window_requires_target_identity_stable() {
        let mut state = BaselineWindowState::new(config());

        for idx in 0..10 {
            let elapsed_ms = (idx + 1) * 3_000;
            let task = if idx == 8 { 8 } else { 7 };
            let records = vec![record(elapsed_ms, task, 10, TaskClass::Game)];
            state.observe_interval(
                (elapsed_ms as u128) * 1_000_000,
                elapsed_ms,
                &records,
                &DropCountersSnapshot::default(),
                identity(&records),
            );
        }

        let status = state.status(30_000_000_000);

        assert!(!status.is_ready());
        assert!(
            status
                .reasons()
                .iter()
                .any(|reason| reason == "target identity shifted during baseline window")
        );
    }

    #[test]
    fn baseline_window_ignores_unscored_classes_for_minimums() {
        let mut state = BaselineWindowState::new(config());

        for idx in 0..10 {
            let elapsed_ms = (idx + 1) * 3_000;
            let records = vec![record(elapsed_ms, 7, 100, TaskClass::Compositor)];
            state.observe_interval(
                (elapsed_ms as u128) * 1_000_000,
                elapsed_ms,
                &records,
                &DropCountersSnapshot::default(),
                identity(&records),
            );
        }

        let status = state.status(30_000_000_000);

        assert!(!status.is_ready());
        assert!(
            status
                .reasons()
                .iter()
                .any(|reason| reason == "zero scored tasks")
        );
    }

    #[test]
    fn baseline_window_config_defaults_match_policy() {
        let config = BaselineWindowConfig::default();

        assert_eq!(config.baseline_window_seconds, 30);
        assert_eq!(config.baseline_window_ms(), 30_000);
        assert_eq!(config.min_scored_intervals, 10);
        assert_eq!(config.min_scored_samples, 100);
    }

    #[test]
    fn baseline_window_progress_reset_and_accumulator_are_observable() {
        let mut state = BaselineWindowState::new(config());
        assert_eq!(state.config().baseline_window_ms(), 30_000);

        let drop_counters = DropCountersSnapshot {
            wakeup_data_insert_failed: 2,
            wakeup_data_stale_entries: 0,
            wakeup_data_replaced_entries: 0,
            wakeup_data_consumed_read_failed: 0,
            ringbuf_reserve_failed: 0,
            irq_start_times_insert_failed: 0,
            block_start_insert_failed: 0,
            block_fallback_key_collisions: 0,
            cpu_accounting_untracked: 0,
            block_zero_keys: 0,
            drm_fence_missing_start: 0,
        };
        let records = vec![record(1_000, 7, 10, TaskClass::Game)];
        let status = state.observe_interval(
            1_000_000_000,
            1_000,
            &records,
            &drop_counters,
            identity(&records),
        );

        match status {
            BaselineWindowStatus::Collecting {
                elapsed_ms,
                scored_intervals,
                scored_samples,
                scored_task_count,
                drop_counter_total,
                reasons,
            } => {
                assert_eq!(elapsed_ms, 0);
                assert_eq!(scored_intervals, 1);
                assert_eq!(scored_samples, 10);
                assert_eq!(scored_task_count, 1);
                assert_eq!(drop_counter_total, 2);
                assert!(
                    reasons
                        .iter()
                        .any(|reason| reason.contains("baseline window not complete"))
                );
            }
            other => panic!("expected collecting baseline status, got {other:?}"),
        }

        let mut accumulator = BaselineWindowAccumulator::default();
        accumulator.push_interval_records(&[
            record(3_000, 3, 1, TaskClass::Game),
            record(1_000, 1, 1, TaskClass::Game),
            record(2_000, 2, 1, TaskClass::Game),
        ]);
        let ordered_elapsed = accumulator
            .ordered_records()
            .into_iter()
            .map(|record| record.elapsed_ms)
            .collect::<Vec<_>>();
        assert_eq!(ordered_elapsed, vec![1_000, 2_000, 3_000]);

        state.reset();
        assert_eq!(state.config(), &config());
        assert!(!state.status(2_000_000_000).is_ready());
    }
}
