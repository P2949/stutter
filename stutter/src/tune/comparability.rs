use std::collections::{BTreeMap, BTreeSet};

use log::warn;
use serde::Serialize;

use super::TuneCandidateSummary;
use crate::{process_tree::TaskClass, recorder, scorer};

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize)]
pub struct TaskIdentity {
    pub class: TaskClass,
    pub process_comm: String,
    pub comm: String,
    pub process_starttime_ticks: Option<u64>,
    pub task_starttime_ticks: Option<u64>,
    pub exe_dev: Option<u64>,
    pub exe_ino: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct TuneCoverageMetrics {
    pub unique_tracked_tasks: usize,
    pub unique_scored_tasks: usize,
    pub active_target_min: usize,
    pub active_target_max: usize,
    pub removed_task_count: usize,
    pub drop_counter_total: u64,
    pub scored_identity_counts: BTreeMap<TaskIdentity, usize>,
}

pub fn check_tune_coverage_comparability(
    grouped: &BTreeMap<String, Vec<TuneCandidateSummary>>,
) -> anyhow::Result<()> {
    check_tune_metric_ratio(
        "unique tracked tasks",
        grouped.values().map(|runs| {
            super::median_u64(
                runs.iter()
                    .map(|result| result.coverage.unique_tracked_tasks as u64)
                    .collect(),
            ) as usize
        }),
    )?;
    check_tune_metric_ratio(
        "unique scored tasks",
        grouped.values().map(|runs| {
            super::median_u64(
                runs.iter()
                    .map(|result| result.coverage.unique_scored_tasks as u64)
                    .collect(),
            ) as usize
        }),
    )?;

    let representatives: Vec<&TuneCandidateSummary> =
        grouped.values().filter_map(|runs| runs.first()).collect();

    if let Some(first) = representatives.first() {
        for other in representatives.iter().skip(1) {
            let common = scored_identity_overlap(
                &first.coverage.scored_identity_counts,
                &other.coverage.scored_identity_counts,
                usize::min,
            );
            let total = scored_identity_overlap(
                &first.coverage.scored_identity_counts,
                &other.coverage.scored_identity_counts,
                usize::max,
            );

            let overlap_ratio = if total > 0 {
                common as f64 / total as f64
            } else {
                1.0
            };

            if overlap_ratio < 0.75 {
                anyhow::bail!(
                    "scored task identity mismatch (overlap={:.1}%); candidates are not comparable (major thread topology shift)",
                    overlap_ratio * 100.0
                );
            }
        }
    }

    check_tune_metric_ratio(
        "active target minimum",
        grouped.values().map(|runs| {
            super::median_u64(
                runs.iter()
                    .map(|result| result.coverage.active_target_min as u64)
                    .collect(),
            ) as usize
        }),
    )?;
    check_tune_metric_ratio(
        "active target maximum",
        grouped.values().map(|runs| {
            super::median_u64(
                runs.iter()
                    .map(|result| result.coverage.active_target_max as u64)
                    .collect(),
            ) as usize
        }),
    )?;

    let profile_removed_medians: Vec<_> = grouped
        .values()
        .map(|runs| {
            super::median_u64(
                runs.iter()
                    .map(|result| result.coverage.removed_task_count as u64)
                    .collect(),
            )
        })
        .collect();

    let min_removed = profile_removed_medians.iter().copied().min().unwrap_or(0);
    let max_removed = profile_removed_medians.iter().copied().max().unwrap_or(0);
    if max_removed > min_removed {
        warn!(
            "tune_candidates_removed_task_counts_differ min_median={} max_median={}",
            min_removed, max_removed
        );
    }

    let max_drops = grouped
        .values()
        .flat_map(|runs| runs.iter().map(|r| r.coverage.drop_counter_total))
        .max()
        .unwrap_or(0);
    if max_drops > 0 {
        warn!("tune_candidates_drop_counters_nonzero max_drops={max_drops}");
    }

    check_tune_sample_comparability(grouped)?;
    check_tune_frame_comparability(grouped)?;

    Ok(())
}

pub fn check_tune_sample_comparability(
    grouped: &BTreeMap<String, Vec<TuneCandidateSummary>>,
) -> anyhow::Result<()> {
    let profile_medians: Vec<_> = grouped
        .values()
        .map(|runs| super::median_u64(runs.iter().map(|r| r.scored_samples).collect()))
        .collect();
    let min_samples = profile_medians.iter().copied().min().unwrap_or(0);
    let max_samples = profile_medians.iter().copied().max().unwrap_or(0);

    if min_samples == 0 && max_samples > 0 {
        anyhow::bail!(
            "tune candidates are not comparable: some profiles gathered no scored samples while others did (max_median_scored_samples={})",
            max_samples
        );
    } else if min_samples > 0 {
        let ratio = (max_samples as f64) / (min_samples as f64);
        if ratio > 2.0 {
            anyhow::bail!(
                "tune candidates are not comparable: median scored sample count varies by more than 2x across profiles (min={} max={} ratio={:.2})",
                min_samples,
                max_samples,
                ratio
            );
        }
    }
    Ok(())
}

pub fn check_tune_frame_comparability(
    grouped: &BTreeMap<String, Vec<TuneCandidateSummary>>,
) -> anyhow::Result<()> {
    let profile_frame_medians: Vec<_> = grouped
        .values()
        .map(|runs| super::median_u64(runs.iter().map(|r| r.frame_count as u64).collect()))
        .collect();
    let min_frames = profile_frame_medians.iter().copied().min().unwrap_or(0);
    let max_frames = profile_frame_medians.iter().copied().max().unwrap_or(0);

    if max_frames > 0 && min_frames == 0 {
        anyhow::bail!(
            "tune candidates are not comparable: some profiles produced MangoHud frame events and some produced none"
        );
    } else if min_frames > 0 {
        let ratio = (max_frames as f64) / (min_frames as f64);
        if ratio > 1.5 {
            warn!(
                "tune_candidates_unbalanced_frames min_median={} max_median={} ratio={:.2}",
                min_frames, max_frames, ratio
            );
        }
    }
    Ok(())
}

pub fn scored_identity_overlap(
    left: &BTreeMap<TaskIdentity, usize>,
    right: &BTreeMap<TaskIdentity, usize>,
    combine: fn(usize, usize) -> usize,
) -> usize {
    left.keys()
        .chain(right.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|identity| {
            combine(
                left.get(identity).copied().unwrap_or(0),
                right.get(identity).copied().unwrap_or(0),
            )
        })
        .sum()
}

pub fn check_tune_metric_ratio(
    label: &str,
    values: impl Iterator<Item = usize>,
) -> anyhow::Result<()> {
    let values = values.collect::<Vec<_>>();
    let min_value = values.iter().copied().min().unwrap_or(0);
    let max_value = values.iter().copied().max().unwrap_or(0);
    if min_value == 0 && max_value > 0 {
        anyhow::bail!(
            "tune candidates are not comparable: {} is zero for some candidates but nonzero for others (max={})",
            label,
            max_value
        );
    }
    if min_value > 0 {
        let ratio = (max_value as f64) / (min_value as f64);
        if ratio > 2.0 {
            anyhow::bail!(
                "tune candidates are not comparable: {} varies by more than 2x across candidates (min={} max={} ratio={:.2})",
                label,
                min_value,
                max_value,
                ratio
            );
        }
    }
    Ok(())
}

pub fn tune_coverage_metrics(
    session: &recorder::SessionFile,
    interval_records: &[recorder::IntervalRecord],
) -> TuneCoverageMetrics {
    let unique_tracked_tasks = session.tasks.len();
    let scored_task_ids = interval_records
        .iter()
        .filter(|record| scorer::class_contributes_to_score(record.class) && record.samples > 0)
        .map(|record| record.task)
        .collect::<BTreeSet<_>>();
    let unique_scored_tasks = scored_task_ids.len();
    let removed_task_count = session
        .tasks
        .iter()
        .filter(|task| !task.active || task.removed_ms.is_some())
        .count();

    let mut active_by_elapsed: BTreeMap<u128, usize> = BTreeMap::new();
    for record in interval_records.iter().filter(|record| record.active) {
        *active_by_elapsed.entry(record.elapsed_ms).or_default() += 1;
    }
    let (active_target_min, active_target_max) = if active_by_elapsed.is_empty() {
        (
            session.active_target_pids_count as usize,
            session.active_target_pids_count as usize,
        )
    } else {
        (
            active_by_elapsed.values().copied().min().unwrap_or(0),
            active_by_elapsed.values().copied().max().unwrap_or(0),
        )
    };

    let tasks_by_tid = session
        .tasks
        .iter()
        .map(|task| (task.task, task))
        .collect::<BTreeMap<_, _>>();
    let mut scored_identity_counts = BTreeMap::<TaskIdentity, usize>::new();
    for tid in scored_task_ids {
        let identity = if let Some(task) = tasks_by_tid.get(&tid) {
            TaskIdentity {
                class: task.class,
                process_comm: task.process_comm.to_string(),
                comm: task.comm.clone(),
                process_starttime_ticks: task.process_starttime_ticks,
                task_starttime_ticks: task.task_starttime_ticks,
                exe_dev: task.exe_dev,
                exe_ino: task.exe_ino,
            }
        } else if let Some(record) = interval_records.iter().find(|record| record.task == tid) {
            TaskIdentity {
                class: record.class,
                process_comm: record.process_comm.to_string(),
                comm: record.comm.clone(),
                process_starttime_ticks: None,
                task_starttime_ticks: None,
                exe_dev: None,
                exe_ino: None,
            }
        } else {
            continue;
        };
        *scored_identity_counts.entry(identity).or_default() += 1;
    }

    TuneCoverageMetrics {
        unique_tracked_tasks,
        unique_scored_tasks,
        active_target_min,
        active_target_max,
        removed_task_count,
        drop_counter_total: if session.block_io_correlation_basis == "request-pointer" {
            session.drop_counters.total()
        } else {
            session.drop_counters.total_excluding_block_io()
        },
        scored_identity_counts,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::process_tree::TaskClass;

    fn mock_candidate(name: &str, coverage: TuneCoverageMetrics) -> TuneCandidateSummary {
        TuneCandidateSummary {
            profile: name.to_string(),
            iteration: 1,
            run_dir: PathBuf::from("."),
            applied_tasks: 0,
            warmup_seconds: 0,
            measure_seconds: 0,
            interval_count: 0,
            samples: 0,
            scored_samples: 0,
            score_total: 0,
            over_1ms: 0,
            over_2ms: 0,
            over_5ms: 0,
            max_latency_ns: 0,
            frame_count: 0,
            frame_max_ms: 0.0,
            frame_p99_ms: 0.0,
            frame_over_16ms: 0,
            frame_over_33ms: 0,
            frame_over_50ms: 0,
            coverage,
            valid: true,
        }
    }

    #[test]
    fn test_check_tune_metric_ratio() {
        let values = vec![10, 15, 20];
        assert!(check_tune_metric_ratio("test", values.into_iter()).is_ok());

        let values = vec![10, 21];
        assert!(check_tune_metric_ratio("test", values.into_iter()).is_err());

        let values = vec![0, 10];
        assert!(check_tune_metric_ratio("test", values.into_iter()).is_err());
    }

    #[test]
    fn test_scored_identity_overlap() {
        let mut left = BTreeMap::new();
        let mut right = BTreeMap::new();

        let id1 = TaskIdentity {
            class: TaskClass::Game,
            process_comm: "p1".to_string(),
            comm: "t1".to_string(),
            process_starttime_ticks: None,
            task_starttime_ticks: None,
            exe_dev: None,
            exe_ino: None,
        };

        let id2 = TaskIdentity {
            class: TaskClass::Game,
            process_comm: "p1".to_string(),
            comm: "t2".to_string(),
            process_starttime_ticks: None,
            task_starttime_ticks: None,
            exe_dev: None,
            exe_ino: None,
        };

        left.insert(id1.clone(), 10);
        left.insert(id2.clone(), 5);

        right.insert(id1.clone(), 8);
        right.insert(id2.clone(), 12);

        assert_eq!(scored_identity_overlap(&left, &right, usize::min), 8 + 5);
        assert_eq!(scored_identity_overlap(&left, &right, usize::max), 10 + 12);
    }

    #[test]
    fn test_check_tune_coverage_comparability_overlap_failure() {
        let id1 = TaskIdentity {
            class: TaskClass::Game,
            process_comm: "p1".to_string(),
            comm: "t1".to_string(),
            process_starttime_ticks: None,
            task_starttime_ticks: None,
            exe_dev: None,
            exe_ino: None,
        };

        let id2 = TaskIdentity {
            class: TaskClass::Game,
            process_comm: "p1".to_string(),
            comm: "t2".to_string(),
            process_starttime_ticks: None,
            task_starttime_ticks: None,
            exe_dev: None,
            exe_ino: None,
        };

        let mut counts1 = BTreeMap::new();
        counts1.insert(id1.clone(), 100);

        let mut counts2 = BTreeMap::new();
        counts2.insert(id2.clone(), 100);

        let mut grouped = BTreeMap::new();
        grouped.insert(
            "p1".to_string(),
            vec![mock_candidate(
                "p1",
                TuneCoverageMetrics {
                    unique_tracked_tasks: 10,
                    unique_scored_tasks: 10,
                    active_target_min: 10,
                    active_target_max: 10,
                    scored_identity_counts: counts1,
                    ..Default::default()
                },
            )],
        );
        grouped.insert(
            "p2".to_string(),
            vec![mock_candidate(
                "p2",
                TuneCoverageMetrics {
                    unique_tracked_tasks: 10,
                    unique_scored_tasks: 10,
                    active_target_min: 10,
                    active_target_max: 10,
                    scored_identity_counts: counts2,
                    ..Default::default()
                },
            )],
        );

        let result = check_tune_coverage_comparability(&grouped);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("identity mismatch")
        );
    }
}
