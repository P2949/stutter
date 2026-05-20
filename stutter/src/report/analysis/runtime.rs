use std::collections::BTreeMap;

use super::*;
use crate::artifacts::{ArtifactKind, artifact_file_name};

pub(crate) fn runtime_slice_analysis_summary(
    session: &SessionFile,
    artifacts: &session_io::RunArtifacts,
) -> RuntimeSliceAnalysisSummary {
    let mut summary = RuntimeSliceAnalysisSummary {
        sample_count: artifacts.runtime_slices.len(),
        available: !artifacts.runtime_slices.is_empty(),
        ..Default::default()
    };

    if !session.config.runtime_slices && session.core.runtime_slice_count == 0 {
        summary.missing_reason = Some("runtime-slice collection was not enabled".to_owned());
    } else if session.config.runtime_slices && session.core.runtime_slice_count == 0 {
        summary.missing_reason = Some("no runtime slice samples were recorded".to_owned());
    } else if artifacts
        .validation
        .missing_optional_files
        .iter()
        .any(|file| file == artifact_file_name(ArtifactKind::RuntimeSlices))
    {
        summary.missing_reason = Some("runtime_slices.json is missing".to_owned());
    } else if session.core.runtime_slice_count > 0 && artifacts.runtime_slices.is_empty() {
        summary.missing_reason =
            Some("runtime slices exist, but none overlapped report correlation windows".to_owned());
    }

    if session.core.runtime_slice_read_errors > 0 {
        summary.data_quality_notes.push(format!(
            "runtime slice sampler had {} read errors",
            session.core.runtime_slice_read_errors
        ));
    }
    if session.core.runtime_slice_skipped_tasks > 0 {
        summary.data_quality_notes.push(format!(
            "runtime slice sampler skipped {} tasks due to runtime_slices_max_tasks",
            session.core.runtime_slice_skipped_tasks
        ));
    }
    if artifacts.runtime_slices.iter().any(|record| {
        matches!(
            record.source,
            crate::metrics::RuntimeSliceSource::ProcStatFallback
        )
    }) {
        summary.data_quality_notes.push(
            "some runtime slices used /proc stat fallback without runqueue wait data".to_owned(),
        );
    }
    if summary.available {
        summary.notes.push(
            "runtime slice context is supporting evidence, not a primary diagnosis by itself"
                .to_owned(),
        );
    }

    for record in artifacts
        .runtime_slices
        .iter()
        .filter(|record| record.valid)
    {
        *summary
            .source_counts
            .entry(record.source.as_str().to_owned())
            .or_insert(0) += 1;
    }

    summary.high_runtime_threads = top_runtime_threads(&artifacts.runtime_slices, true);
    summary.high_wait_threads = top_runtime_threads(&artifacts.runtime_slices, false);
    summary
}

pub(crate) fn top_runtime_threads(
    records: &[crate::metrics::RuntimeSliceRecord],
    by_runtime: bool,
) -> Vec<RuntimeThreadSummary> {
    let mut best_by_task: BTreeMap<u32, RuntimeThreadSummary> = BTreeMap::new();

    for record in records.iter().filter(|record| record.valid) {
        let score = if by_runtime {
            record.runtime_ratio.unwrap_or(0.0)
        } else {
            record.wait_ratio.unwrap_or(0.0)
        };

        if score <= 0.0 {
            continue;
        }

        let candidate = RuntimeThreadSummary {
            task: record.task,
            process_pid: record.process_pid,
            class: record.class,
            comm: record.comm.clone(),
            process_comm: record.process_comm.to_string(),
            max_runtime_ratio: record.runtime_ratio.unwrap_or(0.0),
            max_wait_ratio: record.wait_ratio,
            max_runtime_delta_ns: record.runtime_delta_ns,
            max_runqueue_wait_delta_ns: record.runqueue_wait_delta_ns,
            elapsed_ms: record.elapsed_ms,
        };

        match best_by_task.get(&record.task) {
            Some(existing) => {
                let existing_score = if by_runtime {
                    existing.max_runtime_ratio
                } else {
                    existing.max_wait_ratio.unwrap_or(0.0)
                };
                if score > existing_score {
                    best_by_task.insert(record.task, candidate);
                }
            }
            None => {
                best_by_task.insert(record.task, candidate);
            }
        }
    }

    let mut rows = best_by_task.into_values().collect::<Vec<_>>();
    rows.sort_by(|a, b| {
        let left = if by_runtime {
            a.max_runtime_ratio
        } else {
            a.max_wait_ratio.unwrap_or(0.0)
        };
        let right = if by_runtime {
            b.max_runtime_ratio
        } else {
            b.max_wait_ratio.unwrap_or(0.0)
        };
        right
            .partial_cmp(&left)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.task.cmp(&b.task))
    });
    rows.truncate(10);
    rows
}
