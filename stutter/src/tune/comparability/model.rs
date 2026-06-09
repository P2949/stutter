use std::collections::{BTreeMap, BTreeSet};

use log::warn;
use serde::{Deserialize, Serialize};

use crate::{
    process_tree::TaskClass,
    recorder, scorer,
    tune::{
        model::{RankingConfidence, TuneCandidateSummary, TuneIterationOrder},
        ranking::median_u64,
    },
};

pub const CANDIDATE_ORDER_NOT_COUNTERBALANCED_KIND: &str = "candidate-order-not-counterbalanced";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuneComparabilityWarning {
    pub profile: Option<String>,
    pub kind: String,
    pub message: String,
    pub severity: TuneComparabilitySeverity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TuneComparabilitySeverity {
    Info,
    Warning,
    Reject,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct TaskIdentity {
    pub class: TaskClass,
    pub process_comm: String,
    pub comm: String,
    pub process_starttime_ticks: Option<u64>,
    pub task_starttime_ticks: Option<u64>,
    pub exe_dev: Option<u64>,
    pub exe_ino: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScoredIdentityCount {
    pub identity: TaskIdentity,
    pub count: usize,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TuneCoverageMetrics {
    pub unique_tracked_tasks: usize,
    pub unique_scored_tasks: usize,
    pub active_target_min: usize,
    pub active_target_max: usize,
    pub removed_task_count: usize,
    pub drop_counter_total: u64,

    #[serde(default)]
    pub scored_identity_counts: Vec<ScoredIdentityCount>,
}

pub fn scored_identity_counts_to_map(
    counts: &[ScoredIdentityCount],
) -> BTreeMap<TaskIdentity, usize> {
    let mut map = BTreeMap::new();
    for item in counts {
        *map.entry(item.identity.clone()).or_default() += item.count;
    }
    map
}

pub fn scored_identity_map_to_counts(
    map: BTreeMap<TaskIdentity, usize>,
) -> Vec<ScoredIdentityCount> {
    map.into_iter()
        .map(|(identity, count)| ScoredIdentityCount { identity, count })
        .collect()
}

pub fn check_tune_coverage_comparability(
    grouped: &BTreeMap<String, Vec<TuneCandidateSummary>>,
) -> anyhow::Result<()> {
    check_tune_metric_ratio(
        "unique tracked tasks",
        grouped.values().map(|runs| {
            median_u64(
                runs.iter()
                    .map(|result| result.coverage.unique_tracked_tasks as u64)
                    .collect(),
            ) as usize
        }),
    )?;
    check_tune_metric_ratio(
        "unique scored tasks",
        grouped.values().map(|runs| {
            median_u64(
                runs.iter()
                    .map(|result| result.coverage.unique_scored_tasks as u64)
                    .collect(),
            ) as usize
        }),
    )?;

    let representatives: Vec<&TuneCandidateSummary> =
        grouped.values().filter_map(|runs| runs.first()).collect();

    if let Some(first) = representatives.first() {
        let first_map = scored_identity_counts_to_map(&first.coverage.scored_identity_counts);
        for other in representatives.iter().skip(1) {
            let other_map = scored_identity_counts_to_map(&other.coverage.scored_identity_counts);
            let common = scored_identity_overlap(&first_map, &other_map, usize::min);
            let total = scored_identity_overlap(&first_map, &other_map, usize::max);

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
            median_u64(
                runs.iter()
                    .map(|result| result.coverage.active_target_min as u64)
                    .collect(),
            ) as usize
        }),
    )?;
    check_tune_metric_ratio(
        "active target maximum",
        grouped.values().map(|runs| {
            median_u64(
                runs.iter()
                    .map(|result| result.coverage.active_target_max as u64)
                    .collect(),
            ) as usize
        }),
    )?;

    let profile_removed_medians: Vec<_> = grouped
        .values()
        .map(|runs| {
            median_u64(
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

pub fn tune_comparability_warnings(
    grouped: &BTreeMap<String, Vec<TuneCandidateSummary>>,
) -> Vec<TuneComparabilityWarning> {
    let mut warnings = Vec::new();

    push_median_ratio_warning(
        &mut warnings,
        "scored-sample-count-mismatch",
        "median scored sample count differs by more than 20% between profiles",
        grouped.iter().map(|(profile, runs)| {
            (
                profile,
                median_u64(runs.iter().map(|run| run.scored_samples).collect()),
            )
        }),
        TuneComparabilitySeverity::Warning,
    );
    push_median_ratio_warning(
        &mut warnings,
        "frame-count-mismatch",
        "median frame count differs by more than 20% between profiles",
        grouped.iter().map(|(profile, runs)| {
            (
                profile,
                median_u64(runs.iter().map(|run| run.frame_count as u64).collect()),
            )
        }),
        TuneComparabilitySeverity::Warning,
    );

    let frame_medians = grouped
        .iter()
        .map(|(profile, runs)| {
            (
                profile.as_str(),
                median_u64(runs.iter().map(|run| run.frame_count as u64).collect()),
            )
        })
        .collect::<Vec<_>>();
    if frame_medians.iter().any(|(_, count)| *count == 0)
        && frame_medians.iter().any(|(_, count)| *count > 0)
    {
        for (profile, _) in frame_medians.iter().filter(|(_, count)| *count == 0) {
            warnings.push(TuneComparabilityWarning {
                profile: Some((*profile).to_owned()),
                kind: "missing-frame-data".to_owned(),
                message: "profile has zero median frame count while another profile has frames"
                    .to_owned(),
                severity: TuneComparabilitySeverity::Reject,
            });
        }
    }

    for (profile, runs) in grouped {
        let max_drops = runs
            .iter()
            .map(|run| run.coverage.drop_counter_total)
            .max()
            .unwrap_or(0);
        if max_drops > 0 {
            warnings.push(TuneComparabilityWarning {
                profile: Some(profile.clone()),
                kind: "drop-counters-nonzero".to_owned(),
                message: format!("candidate had non-zero drop counters (max={max_drops})"),
                severity: TuneComparabilitySeverity::Warning,
            });
        }

        if runs.iter().any(|run| run.applied_tasks == 0) {
            warnings.push(TuneComparabilityWarning {
                profile: Some(profile.clone()),
                kind: "applied-task-count-zero".to_owned(),
                message: "candidate applied profile to zero tasks".to_owned(),
                severity: TuneComparabilitySeverity::Warning,
            });
        }
    }

    let valid_counts = grouped
        .iter()
        .map(|(profile, runs)| {
            (
                profile.as_str(),
                runs.iter().filter(|run| run.valid).count() as u64,
            )
        })
        .collect::<Vec<_>>();
    let min_valid = valid_counts
        .iter()
        .map(|(_, count)| *count)
        .min()
        .unwrap_or(0);
    let max_valid = valid_counts
        .iter()
        .map(|(_, count)| *count)
        .max()
        .unwrap_or(0);
    if min_valid != max_valid {
        for (profile, count) in valid_counts {
            warnings.push(TuneComparabilityWarning {
                profile: Some(profile.to_owned()),
                kind: "valid-run-count-mismatch".to_owned(),
                message: format!(
                    "valid run count differs between profiles (profile_valid_runs={count}, min={min_valid}, max={max_valid})"
                ),
                severity: TuneComparabilitySeverity::Warning,
            });
        }
    }

    warnings
}

pub fn tune_candidate_order_warnings(
    candidate_order: &[TuneIterationOrder],
) -> Vec<TuneComparabilityWarning> {
    let mut warnings = Vec::new();

    if candidate_order.is_empty() {
        warnings.push(TuneComparabilityWarning {
            profile: None,
            kind: "candidate-order-metadata-missing".to_owned(),
            message: "candidate order metadata is missing; unable to assess counterbalancing"
                .to_owned(),
            severity: TuneComparabilitySeverity::Warning,
        });
        return warnings;
    }

    // Check if every iteration used the exact same order.
    if candidate_order
        .first()
        .map(|first| {
            candidate_order
                .iter()
                .all(|order| order.profiles.as_slice() == first.profiles.as_slice())
        })
        .unwrap_or(false)
    {
        warnings.push(TuneComparabilityWarning {
            profile: None,
            kind: CANDIDATE_ORDER_NOT_COUNTERBALANCED_KIND.to_owned(),
            message: "all iterations used the same candidate order; profile effect may be confounded with order effect".to_owned(),
            severity: TuneComparabilitySeverity::Warning,
        });
        return warnings;
    }

    // Count how often each profile appears in the first position.
    let mut first_counts = BTreeMap::new();
    let mut total_with_first = 0usize;
    for order in candidate_order.iter() {
        if let Some(first) = order.profiles.first() {
            *first_counts.entry(first.clone()).or_insert(0usize) += 1usize;
            total_with_first += 1;
        }
    }

    if total_with_first > 0 {
        // Check the gap between the most common first-position profile and the
        // runner-up. Alternating sequences with an odd number of runs will
        // naturally produce a 1-count margin (e.g. 2 vs 1 for 3 runs), which is
        // acceptable and should not trigger a warning. Flag only when the margin
        // is 2 or more, indicating a clear imbalance.
        let mut counts: Vec<usize> = first_counts.values().copied().collect();
        counts.sort_unstable_by(|a, b| b.cmp(a));
        let max = counts.first().copied().unwrap_or(0);
        let second = counts.get(1).copied().unwrap_or(0);
        if max >= second + 2 {
            // Significant imbalance detected.
            let (profile, count) = first_counts
                .into_iter()
                .max_by_key(|(_, c)| *c)
                .unwrap_or_default();
            warnings.push(TuneComparabilityWarning {
                profile: Some(profile.clone()),
                kind: CANDIDATE_ORDER_NOT_COUNTERBALANCED_KIND.to_owned(),
                message: format!(
                    "profile '{}' appears first in {}/{} iterations; candidate order may be imbalanced",
                    profile, count, total_with_first
                ),
                severity: TuneComparabilitySeverity::Warning,
            });
        }
    }

    warnings
}

pub fn has_candidate_order_counterbalance_warning(warnings: &[TuneComparabilityWarning]) -> bool {
    warnings
        .iter()
        .any(|warning| warning.kind == CANDIDATE_ORDER_NOT_COUNTERBALANCED_KIND)
}

pub fn ranking_confidence_after_comparability_warnings(
    confidence: RankingConfidence,
    warnings: &[TuneComparabilityWarning],
) -> RankingConfidence {
    if !has_candidate_order_counterbalance_warning(warnings) {
        return confidence;
    }

    match confidence {
        RankingConfidence::High | RankingConfidence::Medium => RankingConfidence::Low,
        RankingConfidence::Low | RankingConfidence::Unstable => confidence,
    }
}

// helper removed: two_profile_candidate_order_is_fixed was unused and caused
// `dead_code` lint failures under CI's `-D warnings` policy.

fn push_median_ratio_warning<'a>(
    warnings: &mut Vec<TuneComparabilityWarning>,
    kind: &str,
    message: &str,
    values: impl Iterator<Item = (&'a String, u64)>,
    severity: TuneComparabilitySeverity,
) {
    let values = values.collect::<Vec<_>>();
    let min = values.iter().map(|(_, value)| *value).min().unwrap_or(0);
    let max = values.iter().map(|(_, value)| *value).max().unwrap_or(0);

    if min == 0 && max > 0 {
        for (profile, value) in values {
            warnings.push(TuneComparabilityWarning {
                profile: Some(profile.clone()),
                kind: kind.to_owned(),
                message: format!("{message} (profile_median={value}, min={min}, max={max})"),
                severity: TuneComparabilitySeverity::Reject,
            });
        }
    } else if min > 0 {
        let ratio = max as f64 / min as f64;
        if ratio > 1.20 {
            for (profile, value) in values {
                warnings.push(TuneComparabilityWarning {
                    profile: Some(profile.clone()),
                    kind: kind.to_owned(),
                    message: format!(
                        "{message} (profile_median={value}, min={min}, max={max}, ratio={ratio:.2})"
                    ),
                    severity: severity.clone(),
                });
            }
        }
    }
}

pub fn check_tune_sample_comparability(
    grouped: &BTreeMap<String, Vec<TuneCandidateSummary>>,
) -> anyhow::Result<()> {
    let profile_medians: Vec<_> = grouped
        .values()
        .map(|runs| median_u64(runs.iter().map(|r| r.scored_samples).collect()))
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
        .map(|runs| median_u64(runs.iter().map(|r| r.frame_count as u64).collect()))
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

    let mut active_by_elapsed: BTreeMap<u64, usize> = BTreeMap::new();
    for record in interval_records.iter().filter(|record| record.active) {
        *active_by_elapsed.entry(record.elapsed_ms).or_default() += 1;
    }
    let (active_target_min, active_target_max) = if active_by_elapsed.is_empty() {
        (
            session.core.active_target_pids_count as usize,
            session.core.active_target_pids_count as usize,
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
                process_comm: task.process_comm.clone(),
                comm: task.comm.clone(),
                process_starttime_ticks: task.process_starttime_ticks,
                task_starttime_ticks: task.task_starttime_ticks,
                exe_dev: task.exe_dev,
                exe_ino: task.exe_ino,
            }
        } else if let Some(record) = interval_records.iter().find(|record| record.task == tid) {
            TaskIdentity {
                class: record.class,
                process_comm: record.process_comm.clone(),
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
        drop_counter_total: if session.core.block_io_correlation_basis == "request-pointer" {
            session.core.drop_counters.total()
        } else {
            session.core.drop_counters.total_excluding_block_io()
        },
        scored_identity_counts: scored_identity_map_to_counts(scored_identity_counts),
    }
}
