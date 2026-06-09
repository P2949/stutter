use std::collections::{BTreeMap, BTreeSet};

use crate::model::ReportModel;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReportDiff {
    pub changed: bool,
    pub score_changed: bool,
    pub p95_latency_changed: bool,
    pub p99_latency_changed: bool,
    pub top_culprit_changed: bool,
    pub data_quality_changed: bool,
}

impl ReportDiff {
    pub const fn unchanged() -> Self {
        Self {
            changed: false,
            score_changed: false,
            p95_latency_changed: false,
            p99_latency_changed: false,
            top_culprit_changed: false,
            data_quality_changed: false,
        }
    }

    pub const fn with_changes() -> Self {
        Self {
            changed: true,
            score_changed: false,
            p95_latency_changed: false,
            p99_latency_changed: false,
            top_culprit_changed: false,
            data_quality_changed: false,
        }
    }

    pub const fn has_changes(&self) -> bool {
        self.changed
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportDiffTask<I> {
    pub identity: I,
    pub samples: u64,
    pub p95_ns: u64,
    pub p99_ns: u64,
    pub max_ns: u64,
    pub over_1ms: u64,
    pub over_2ms: u64,
    pub over_5ms: u64,
    pub scored: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportTaskDelta<I> {
    pub identity: I,
    pub baseline_samples: u64,
    pub current_samples: u64,
    pub baseline_p95_ns: u64,
    pub current_p95_ns: u64,
    pub delta_p95_ns: i64,
    pub baseline_p99_ns: u64,
    pub current_p99_ns: u64,
    pub delta_p99_ns: i64,
    pub baseline_max_ns: u64,
    pub current_max_ns: u64,
    pub delta_max_ns: i64,
    pub baseline_over_1ms: u64,
    pub current_over_1ms: u64,
    pub delta_over_1ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportNumericDelta {
    pub baseline: i64,
    pub current: i64,
    pub delta: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportValueChange<T> {
    pub baseline: Option<T>,
    pub current: Option<T>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportRunMetadata<I> {
    pub score: Option<i64>,
    pub p95_latency_ns: Option<u64>,
    pub p99_latency_ns: Option<u64>,
    pub top_culprit: Option<I>,
    pub data_quality_level: Option<String>,
}

impl<I> Default for ReportRunMetadata<I> {
    fn default() -> Self {
        Self {
            score: None,
            p95_latency_ns: None,
            p99_latency_ns: None,
            top_culprit: None,
            data_quality_level: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportRunDiffInput<I> {
    pub baseline: ReportRunMetadata<I>,
    pub current: ReportRunMetadata<I>,
    pub baseline_tasks: Vec<ReportDiffTask<I>>,
    pub current_tasks: Vec<ReportDiffTask<I>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportRunDiff<I> {
    pub compared_tasks: usize,
    pub score_delta: Option<ReportNumericDelta>,
    pub p95_latency_delta_ns: Option<ReportNumericDelta>,
    pub p99_latency_delta_ns: Option<ReportNumericDelta>,
    pub top_culprit_change: Option<ReportValueChange<I>>,
    pub data_quality_change: Option<ReportValueChange<String>>,
    pub worst_p95_regression: Option<ReportTaskDelta<I>>,
    pub worst_p99_regression: Option<ReportTaskDelta<I>>,
    pub worst_max_regression: Option<ReportTaskDelta<I>>,
    pub regressions: Vec<ReportTaskDelta<I>>,
    pub improvements: Vec<ReportTaskDelta<I>>,
    pub new_scored_tasks: Vec<ReportDiffTask<I>>,
    pub new_tasks: Vec<ReportDiffTask<I>>,
    pub removed_tasks: Vec<ReportDiffTask<I>>,
}

pub fn diff_report_models(baseline: &ReportModel, current: &ReportModel) -> ReportDiff {
    let score_changed = baseline.score != current.score;
    let p95_latency_changed = baseline.p95_latency_ns != current.p95_latency_ns;
    let p99_latency_changed = baseline.p99_latency_ns != current.p99_latency_ns;
    let top_culprit_changed = baseline.top_culprit != current.top_culprit;
    let data_quality_changed = baseline.data_quality.as_ref().map(|quality| quality.level)
        != current.data_quality.as_ref().map(|quality| quality.level);
    let changed = baseline.run_id != current.run_id
        || score_changed
        || p95_latency_changed
        || p99_latency_changed
        || top_culprit_changed
        || data_quality_changed;

    ReportDiff {
        changed,
        score_changed,
        p95_latency_changed,
        p99_latency_changed,
        top_culprit_changed,
        data_quality_changed,
    }
}

pub fn diff_report_task_sets<I>(
    baseline_tasks: Vec<ReportDiffTask<I>>,
    current_tasks: Vec<ReportDiffTask<I>>,
) -> ReportRunDiff<I>
where
    I: Ord + Clone,
{
    diff_report_runs(ReportRunDiffInput {
        baseline: ReportRunMetadata::default(),
        current: ReportRunMetadata::default(),
        baseline_tasks,
        current_tasks,
    })
}

pub fn diff_report_runs<I>(input: ReportRunDiffInput<I>) -> ReportRunDiff<I>
where
    I: Ord + Clone,
{
    let baseline_tasks = task_map(input.baseline_tasks);
    let current_tasks = task_map(input.current_tasks);
    let mut compared_tasks = 0;
    let mut regressions = Vec::new();
    let mut improvements = Vec::new();

    for (identity, baseline_task) in &baseline_tasks {
        if let Some(current_task) = current_tasks.get(identity) {
            compared_tasks += 1;
            let delta = task_delta(identity.clone(), baseline_task, current_task);
            if delta.delta_max_ns > 0
                || delta.delta_p99_ns > 0
                || delta.delta_p95_ns > 0
                || delta.delta_over_1ms > 0
            {
                regressions.push(delta);
            } else if delta.delta_max_ns < 0
                || delta.delta_p99_ns < 0
                || delta.delta_p95_ns < 0
                || delta.delta_over_1ms < 0
            {
                improvements.push(delta);
            }
        }
    }

    regressions.sort_by(|left, right| compare_regression(right, left));
    improvements.sort_by(compare_improvement);

    let worst_p95_regression = regressions
        .iter()
        .filter(|delta| delta.delta_p95_ns > 0)
        .max_by_key(|delta| delta.delta_p95_ns)
        .cloned();
    let worst_p99_regression = regressions
        .iter()
        .filter(|delta| delta.delta_p99_ns > 0)
        .max_by_key(|delta| delta.delta_p99_ns)
        .cloned();
    let worst_max_regression = regressions
        .iter()
        .filter(|delta| delta.delta_max_ns > 0)
        .max_by_key(|delta| delta.delta_max_ns)
        .cloned();

    let current_keys = current_tasks.keys().cloned().collect::<BTreeSet<_>>();
    let baseline_keys = baseline_tasks.keys().cloned().collect::<BTreeSet<_>>();

    let mut new_tasks = current_keys
        .difference(&baseline_keys)
        .filter_map(|identity| current_tasks.get(identity).cloned())
        .collect::<Vec<_>>();
    new_tasks.sort_by_key(|task| std::cmp::Reverse(task.max_ns));

    let new_scored_tasks = new_tasks
        .iter()
        .filter(|task| task.scored)
        .cloned()
        .collect::<Vec<_>>();

    let mut removed_tasks = baseline_keys
        .difference(&current_keys)
        .filter_map(|identity| baseline_tasks.get(identity).cloned())
        .collect::<Vec<_>>();
    removed_tasks.sort_by_key(|task| std::cmp::Reverse(task.max_ns));

    ReportRunDiff {
        compared_tasks,
        score_delta: numeric_delta(input.baseline.score, input.current.score),
        p95_latency_delta_ns: numeric_delta_u64(
            input.baseline.p95_latency_ns,
            input.current.p95_latency_ns,
        ),
        p99_latency_delta_ns: numeric_delta_u64(
            input.baseline.p99_latency_ns,
            input.current.p99_latency_ns,
        ),
        top_culprit_change: value_change(input.baseline.top_culprit, input.current.top_culprit),
        data_quality_change: value_change(
            input.baseline.data_quality_level,
            input.current.data_quality_level,
        ),
        worst_p95_regression,
        worst_p99_regression,
        worst_max_regression,
        regressions,
        improvements,
        new_scored_tasks,
        new_tasks,
        removed_tasks,
    }
}

fn task_map<I>(tasks: Vec<ReportDiffTask<I>>) -> BTreeMap<I, ReportDiffTask<I>>
where
    I: Ord + Clone,
{
    tasks
        .into_iter()
        .map(|task| (task.identity.clone(), task))
        .collect()
}

fn task_delta<I>(
    identity: I,
    baseline: &ReportDiffTask<I>,
    current: &ReportDiffTask<I>,
) -> ReportTaskDelta<I> {
    ReportTaskDelta {
        identity,
        baseline_samples: baseline.samples,
        current_samples: current.samples,
        baseline_p95_ns: baseline.p95_ns,
        current_p95_ns: current.p95_ns,
        delta_p95_ns: current.p95_ns as i64 - baseline.p95_ns as i64,
        baseline_p99_ns: baseline.p99_ns,
        current_p99_ns: current.p99_ns,
        delta_p99_ns: current.p99_ns as i64 - baseline.p99_ns as i64,
        baseline_max_ns: baseline.max_ns,
        current_max_ns: current.max_ns,
        delta_max_ns: current.max_ns as i64 - baseline.max_ns as i64,
        baseline_over_1ms: baseline.over_1ms,
        current_over_1ms: current.over_1ms,
        delta_over_1ms: current.over_1ms as i64 - baseline.over_1ms as i64,
    }
}

fn compare_regression<I>(
    left: &ReportTaskDelta<I>,
    right: &ReportTaskDelta<I>,
) -> std::cmp::Ordering
where
    I: Ord,
{
    (
        left.delta_max_ns.max(0),
        left.delta_p99_ns.max(0),
        left.delta_p95_ns.max(0),
        left.delta_over_1ms.max(0),
        &left.identity,
    )
        .cmp(&(
            right.delta_max_ns.max(0),
            right.delta_p99_ns.max(0),
            right.delta_p95_ns.max(0),
            right.delta_over_1ms.max(0),
            &right.identity,
        ))
}

fn compare_improvement<I>(
    left: &ReportTaskDelta<I>,
    right: &ReportTaskDelta<I>,
) -> std::cmp::Ordering
where
    I: Ord,
{
    (
        left.delta_max_ns.min(0),
        left.delta_p99_ns.min(0),
        left.delta_p95_ns.min(0),
        left.delta_over_1ms.min(0),
        &left.identity,
    )
        .cmp(&(
            right.delta_max_ns.min(0),
            right.delta_p99_ns.min(0),
            right.delta_p95_ns.min(0),
            right.delta_over_1ms.min(0),
            &right.identity,
        ))
}

fn numeric_delta(baseline: Option<i64>, current: Option<i64>) -> Option<ReportNumericDelta> {
    let (baseline, current) = baseline.zip(current)?;
    (baseline != current).then_some(ReportNumericDelta {
        baseline,
        current,
        delta: current.saturating_sub(baseline),
    })
}

fn numeric_delta_u64(baseline: Option<u64>, current: Option<u64>) -> Option<ReportNumericDelta> {
    numeric_delta(
        baseline.map(|value| value as i64),
        current.map(|value| value as i64),
    )
}

fn value_change<T>(baseline: Option<T>, current: Option<T>) -> Option<ReportValueChange<T>>
where
    T: Eq,
{
    (baseline != current).then_some(ReportValueChange { baseline, current })
}

#[cfg(test)]
mod tests {
    use stutter_core::ids::RunId;

    use super::{
        ReportDiffTask, ReportRunDiffInput, ReportRunMetadata, diff_report_models,
        diff_report_runs, diff_report_task_sets,
    };
    use crate::model::{DataQualityLevel, DataQualitySummary, ReportModel};

    #[test]
    fn diff_reports_no_change_for_equal_models() {
        let model = ReportModel::new().with_run_id(RunId::new("run-001"));

        let diff = diff_report_models(&model, &model);

        assert!(!diff.has_changes());
    }

    #[test]
    fn diff_reports_change_for_different_models() {
        let baseline = ReportModel::new().with_run_id(RunId::new("run-001"));
        let current = ReportModel::new().with_run_id(RunId::new("run-002"));

        let diff = diff_report_models(&baseline, &current);

        assert!(diff.has_changes());
    }

    #[test]
    fn model_diff_reports_score_latency_culprit_and_quality_changes() {
        let mut baseline = ReportModel::new().with_run_id(RunId::new("run-001"));
        baseline.score = Some(10.0);
        baseline.p95_latency_ns = Some(1_000);
        baseline.p99_latency_ns = Some(2_000);
        baseline.top_culprit = Some("render".to_owned());
        baseline.data_quality = Some(data_quality(DataQualityLevel::High));

        let mut current = baseline.clone();
        current.score = Some(20.0);
        current.p95_latency_ns = Some(3_000);
        current.p99_latency_ns = Some(4_000);
        current.top_culprit = Some("shader".to_owned());
        current.data_quality = Some(data_quality(DataQualityLevel::Low));

        let diff = diff_report_models(&baseline, &current);

        assert!(diff.has_changes());
        assert!(diff.score_changed);
        assert!(diff.p95_latency_changed);
        assert!(diff.p99_latency_changed);
        assert!(diff.top_culprit_changed);
        assert!(diff.data_quality_changed);
    }

    #[test]
    fn run_diff_reports_task_regressions_improvements_and_task_churn() {
        let diff = diff_report_task_sets(
            vec![
                task("render", 10, 20, 30, true),
                task("audio", 40, 50, 60, true),
                task("removed", 1, 2, 100, false),
            ],
            vec![
                task("render", 30, 60, 90, true),
                task("audio", 10, 20, 30, true),
                task("new", 10, 20, 120, true),
            ],
        );

        assert_eq!(diff.compared_tasks, 2);
        assert_eq!(diff.regressions[0].identity, "render");
        assert_eq!(diff.improvements[0].identity, "audio");
        assert_eq!(
            diff.worst_p95_regression.as_ref().unwrap().identity,
            "render"
        );
        assert_eq!(
            diff.worst_p99_regression.as_ref().unwrap().identity,
            "render"
        );
        assert_eq!(
            diff.worst_max_regression.as_ref().unwrap().identity,
            "render"
        );
        assert_eq!(diff.new_scored_tasks[0].identity, "new");
        assert_eq!(diff.removed_tasks[0].identity, "removed");
    }

    #[test]
    fn run_diff_reports_score_latency_culprit_and_quality_metadata() {
        let diff = diff_report_runs(ReportRunDiffInput {
            baseline: ReportRunMetadata {
                score: Some(100),
                p95_latency_ns: Some(1_000),
                p99_latency_ns: Some(2_000),
                top_culprit: Some("render"),
                data_quality_level: Some("High".to_owned()),
            },
            current: ReportRunMetadata {
                score: Some(130),
                p95_latency_ns: Some(1_500),
                p99_latency_ns: Some(2_500),
                top_culprit: Some("shader"),
                data_quality_level: Some("Medium".to_owned()),
            },
            baseline_tasks: Vec::new(),
            current_tasks: Vec::new(),
        });

        assert_eq!(diff.score_delta.unwrap().delta, 30);
        assert_eq!(diff.p95_latency_delta_ns.unwrap().delta, 500);
        assert_eq!(diff.p99_latency_delta_ns.unwrap().delta, 500);
        assert_eq!(diff.top_culprit_change.unwrap().current, Some("shader"));
        assert_eq!(
            diff.data_quality_change.unwrap().current,
            Some("Medium".to_owned())
        );
    }

    fn task(
        identity: &'static str,
        p95_ns: u64,
        p99_ns: u64,
        max_ns: u64,
        scored: bool,
    ) -> ReportDiffTask<&'static str> {
        ReportDiffTask {
            identity,
            samples: 1,
            p95_ns,
            p99_ns,
            max_ns,
            over_1ms: p99_ns / 10,
            over_2ms: 0,
            over_5ms: 0,
            scored,
        }
    }

    fn data_quality(level: DataQualityLevel) -> DataQualitySummary {
        DataQualitySummary {
            level,
            reasons: Vec::new(),
            missing_optional_files: Vec::new(),
            validation_errors: Vec::new(),
            validation_warnings: Vec::new(),
            probe_activation_warnings: Vec::new(),
            schema_version: 1,
            expected_schema_version: 1,
            event_stream_write_errors: 0,
            spike_events_truncated: false,
            spike_events_retained_count: 0,
            spike_events_dropped_count: 0,
            interval_record_count: 0,
            active_target_pids_count: 0,
            drop_counters_nonzero: false,
            percentile_scope_counts: Default::default(),
            block_io_correlation_basis: "none".to_owned(),
            block_io_correlation_confidence: "high".to_owned(),
            block_io_correlation_warning: None,
            frame_timestamp_alignment: "none".to_owned(),
            cpu_perf_requested: false,
            cpu_perf_open_errors: 0,
            cpu_perf_read_errors: 0,
            cpu_perf_skipped_tasks: 0,
        }
    }
}
