//! Spike clustering and wake-graph helpers.
//!
//! Owns spike point flattening, cluster construction, task-count maintenance, and wake graph edges.
//! Does not own pressure timelines, diagnosis explanation, task row formatting, or orchestration.

use std::collections::BTreeMap;

use super::*;
use crate::sched_state::classify_switch_prev_state;

pub(crate) fn spike_cluster_analysis(
    session: &SessionFile,
    spike_events: Option<&[SpikeEvent]>,
    cluster_window_ns: u64,
    top: usize,
    filter_class: Option<TaskClass>,
) -> SpikeClusterAnalysis {
    let (source, mut points) = match spike_events {
        Some(spike_events) => (
            SpikeClusterSource::SpikeEvents,
            flatten_spike_events(session, spike_events),
        ),
        None => (
            SpikeClusterSource::TopSpikesFallback,
            flatten_top_spikes(session),
        ),
    };

    if let Some(class) = filter_class {
        points.retain(|p| p.class == class);
    }

    let source_count = points.len();

    SpikeClusterAnalysis {
        source,
        source_count,
        clusters: spike_clusters_from_points(points, cluster_window_ns, top),
    }
}

pub(crate) fn spike_clusters_from_points(
    mut points: Vec<SpikePoint>,
    cluster_window_ns: u64,
    top: usize,
) -> Vec<SpikeCluster> {
    points.sort_by_key(|point| point.switch_ns);

    let mut candidates = BinaryHeap::new();
    let mut task_counts = BTreeMap::<u32, usize>::new();
    let mut max_latency_candidates = std::collections::VecDeque::<usize>::new();
    let mut left_idx = 0;

    for right_idx in 0..points.len() {
        *task_counts.entry(points[right_idx].task).or_default() += 1;

        while max_latency_candidates
            .back()
            .is_some_and(|idx| points[*idx].latency_ns <= points[right_idx].latency_ns)
        {
            max_latency_candidates.pop_back();
        }
        max_latency_candidates.push_back(right_idx);

        while left_idx <= right_idx
            && points[right_idx]
                .switch_ns
                .saturating_sub(points[left_idx].switch_ns)
                > cluster_window_ns
        {
            decrement_task_count(&mut task_counts, points[left_idx].task);
            if max_latency_candidates.front() == Some(&left_idx) {
                max_latency_candidates.pop_front();
            }
            left_idx += 1;
        }

        if task_counts.len() >= MIN_CLUSTER_TASKS {
            let max_latency_ns = max_latency_candidates
                .front()
                .map(|idx| points[*idx].latency_ns)
                .unwrap_or(0);

            let candidate = SpikeClusterCandidate {
                start_idx: left_idx,
                end_idx: right_idx + 1,
                distinct_tasks: task_counts.len(),
                min_switch_ns: points[left_idx].switch_ns,
                max_switch_ns: points[right_idx].switch_ns,
                max_latency_ns,
            };

            if candidates.len() < MAX_CLUSTER_CANDIDATES {
                candidates.push(std::cmp::Reverse(candidate));
            } else if let Some(mut worst) = candidates.peek_mut()
                && candidate > worst.0
            {
                *worst = std::cmp::Reverse(candidate);
            }
        }
    }

    let mut candidates_vec: Vec<_> = candidates.into_iter().map(|r| r.0).collect();
    candidates_vec.sort_by(|a, b| b.cmp(a));

    let mut selected_candidates = Vec::new();
    let max_selected = top.saturating_mul(4).min(MAX_CLUSTER_CANDIDATES);

    // Sweep-line: track selected intervals by max_switch_ns in a BTreeSet
    // for O(log n) overlap checking instead of O(n) per candidate.
    let mut selected_intervals: BTreeSet<(u64, u64)> = BTreeSet::new(); // (max_switch_ns, min_switch_ns)

    for candidate in candidates_vec {
        // Check for overlap: we need intervals where
        //   existing.min_switch_ns <= candidate.max_switch_ns AND
        //   existing.max_switch_ns >= candidate.min_switch_ns
        //
        // Since intervals are stored as (max_switch_ns, min_switch_ns),
        // we look for entries whose max_switch_ns >= candidate.min_switch_ns.
        let overlaps = selected_intervals
            .range((candidate.min_switch_ns, 0)..)
            .any(|(_, min_ns)| *min_ns <= candidate.max_switch_ns);

        if !overlaps {
            selected_intervals.insert((candidate.max_switch_ns, candidate.min_switch_ns));
            selected_candidates.push(candidate);
            if selected_candidates.len() >= max_selected {
                break;
            }
        }
    }

    selected_candidates
        .into_iter()
        .map(|candidate| {
            cluster_from_points(
                points[candidate.start_idx..candidate.end_idx].to_vec(),
                candidate.distinct_tasks,
            )
        })
        .collect()
}

pub(crate) fn flatten_spike_events(
    session: &SessionFile,
    spike_events: &[SpikeEvent],
) -> Vec<SpikePoint> {
    spike_events
        .iter()
        .map(|spike| SpikePoint {
            task: spike.task.as_u32(),
            class: spike.class,
            process_pid: spike.process_pid.map(|pid| pid.as_u32()),
            comm: spike.comm.clone(),
            cpu: spike.cpu,
            wakeup_target_cpu: spike.wakeup_target_cpu,
            latency_ns: spike.latency_ns,
            wakeup_ns: spike.wakeup_ns,
            switch_ns: spike.switch_ns,
            target_pending_wakeups: spike.target_pending_wakeups,
            observed_runnable_depth: spike.observed_runnable_depth,
            switch_prev_pid: spike.switch_prev_pid.as_u32(),
            switch_prev_state: spike.switch_prev_state,
            switch_prev_state_label: classify_switch_prev_state(spike.switch_prev_state).to_owned(),
            elapsed_ms: elapsed_ms(session.core.monotonic_start_ns, spike.switch_ns)
                .or(spike.elapsed_ms),
            scx_ops: spike.scx_ops.clone(),
            scx_state: spike.scx_state.clone(),
            waker_tid: spike.waker_tid.as_u32(),
            waker_comm: spike.waker_comm.clone(),
            cause_tags: spike.cause_tags.clone(),
            primary_cause: spike.primary_cause.clone(),
        })
        .collect()
}

pub(crate) fn flatten_top_spikes(session: &SessionFile) -> Vec<SpikePoint> {
    let mut points = Vec::new();

    for task in &session.tasks {
        for spike in &task.top_spikes {
            points.push(spike_point_from_task(
                task,
                spike,
                elapsed_ms(session.core.monotonic_start_ns, spike.switch_ns),
            ));
        }
    }

    points
}

pub(crate) fn spike_point_from_task(
    task: &SessionTask,
    spike: &RecordedSpike,
    elapsed_ms: Option<u64>,
) -> SpikePoint {
    SpikePoint {
        task: task.task,
        class: spike.class,
        process_pid: spike.process_pid,
        comm: task.comm.clone(),
        cpu: spike.cpu,
        wakeup_target_cpu: spike.wakeup_target_cpu,
        switch_prev_pid: spike.switch_prev_pid,
        switch_prev_state: spike.switch_prev_state,
        switch_prev_state_label: classify_switch_prev_state(spike.switch_prev_state).to_owned(),
        latency_ns: spike.latency_ns,
        wakeup_ns: spike.wakeup_ns,
        switch_ns: spike.switch_ns,
        target_pending_wakeups: spike.target_pending_wakeups,
        observed_runnable_depth: spike.observed_runnable_depth,
        elapsed_ms,
        scx_ops: spike.scx_ops.clone(),
        scx_state: spike.scx_state.clone(),
        waker_tid: spike.waker_tid,
        waker_comm: spike.waker_comm.clone(),
        cause_tags: spike.cause_tags.clone(),
        primary_cause: spike.primary_cause.clone(),
    }
}

pub(crate) fn elapsed_ms(monotonic_start_ns: Option<u64>, switch_ns: u64) -> Option<u64> {
    let start_ns = monotonic_start_ns?;
    switch_ns
        .checked_sub(start_ns)
        .map(|elapsed_ns| elapsed_ns / 1_000_000)
}

pub(crate) fn decrement_task_count(task_counts: &mut BTreeMap<u32, usize>, task: u32) {
    let Some(count) = task_counts.get_mut(&task) else {
        return;
    };
    *count -= 1;
    if *count == 0 {
        task_counts.remove(&task);
    }
}

pub(crate) fn cluster_from_points(
    mut points: Vec<SpikePoint>,
    distinct_tasks: usize,
) -> SpikeCluster {
    points.sort_by_key(|point| (point.switch_ns, std::cmp::Reverse(point.latency_ns)));

    let min_switch_ns = points.first().map(|point| point.switch_ns).unwrap_or(0);
    let max_switch_ns = points.last().map(|point| point.switch_ns).unwrap_or(0);
    let max_latency_ns = points
        .iter()
        .map(|point| point.latency_ns)
        .max()
        .unwrap_or(0);

    let wake_graph = build_wake_graph(&points);

    SpikeCluster {
        points,
        distinct_tasks,
        min_switch_ns,
        max_switch_ns,
        max_latency_ns,
        diagnosis: None,
        diagnosis_explanation: None,
        anchor_task: None,
        anchor_class: None,
        anchor_comm: None,
        anchor_kind: None,
        foreground_pid: None,
        foreground_app_id: None,
        foreground_class: None,
        foreground_confidence: None,
        wake_graph,
    }
}

pub(crate) fn build_wake_graph(points: &[SpikePoint]) -> Vec<WakeGraphEdge> {
    let mut edges = BTreeMap::<(u32, String, u32, String), (u64, u64)>::new();

    for point in points {
        if point.waker_tid != 0 {
            let key = (
                point.waker_tid,
                point.waker_comm.clone(),
                point.task,
                point.comm.clone(),
            );
            let entry = edges.entry(key).or_insert((0, 0));
            entry.0 += 1;
            entry.1 = entry.1.max(point.latency_ns);
        }
    }

    let mut result: Vec<_> = edges
        .into_iter()
        .map(
            |((waker_tid, waker_comm, wakee_tid, wakee_comm), (count, max_latency_ns))| {
                WakeGraphEdge {
                    waker_tid,
                    waker_comm,
                    wakee_tid,
                    wakee_comm,
                    count,
                    max_latency_ns,
                }
            },
        )
        .collect();

    // Sort by count desc, then max_latency_ns desc
    result.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| b.max_latency_ns.cmp(&a.max_latency_ns))
    });

    result.truncate(16);
    result
}
