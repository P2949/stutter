//! The resident runtime's own observation of a research attempt: the shared observer the funnel arms on the attested
//! game process and the runtime feeds from the events it already handles (interval records, GPU samples). It counts
//! only what the runtime reports of that exact process, in the controller's canonical histogram layout, and keeps one
//! finished window per attempt for the sealed records the independent verifiers read. It never decides anything and
//! never reaches the controller: a second sensor path or a second optimizer is not created, the existing events are
//! read.

use std::sync::{Arc, Mutex};

use super::{action_child::ChildTarget, child_verify::AttemptProcess};
use crate::{
    autotune::{objective::runnable_signal::is_primary_game_class, rolling_window::RollingWindow},
    metrics::{IntervalRecord, LatencyHistogram, LatencyHistogramBucket},
    recorder::{CpuFreqRecord, FrameEvent, GpuSample},
};

/// Shared between the runtime (which reports its events) and the funnel (which opens and closes attempts).
pub type SharedResident = Arc<Mutex<ResidentObserver>>;

/// The guard facts of one window, read from the production objective-signal builder over the same events the
/// controller's own window holds: nothing here is a second definition of a frame tail or a throttle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuardFacts {
    pub frame_p99_ms: f64,
    pub frame_samples: u64,
    pub foreground_over_5ms: u64,
    pub thermal_degraded: bool,
    pub thermal_throttle_count: u64,
    pub cpu_power_limited: bool,
    pub gpu_power_limited: bool,
}

/// One attempt's finished window.
#[derive(Clone, Debug)]
pub struct AttemptResident {
    pub target: ChildTarget,
    /// The process's cgroup as the kernel showed it when the window opened (empty when unreadable).
    pub cgroup: String,
    pub attempt: Option<u32>,
    pub started_unix: u64,
    pub ended_unix: u64,
    /// The target game's merged runnable-delay population, in the canonical layout, and its maximum.
    pub histogram: Vec<LatencyHistogramBucket>,
    pub max_ns: u64,
    /// Events the sources dropped during the window.
    pub lost_events: u64,
    /// Interval records of the target that were not a valid canonical population (they are not counted).
    pub invalid_intervals: u64,
    /// A game process other than the target appeared in the target's intervals.
    pub target_changed: bool,
    pub cpu_samples: u64,
    pub gpu_samples: u64,
    /// CPU frequency events the runtime saw while the attempt ran.
    pub cpu_freq_samples: u64,
    pub psi_samples: u64,
    /// Interval records of the window that carried no process identity: while any exists, the resource owners are not
    /// all identified (a scheduler interval alone does not say who owns the load).
    pub unattributed_intervals: u64,
    /// Planning passes that considered this process, and those that offered an authorized action: the planner's own
    /// account, from the decision stream the runtime already emits.
    pub evaluations: u64,
    /// Planning passes that ran but denied every authorized action (policy, capability, workload or manual-only),
    /// which is not evidence that no action was indicated.
    pub denied_evaluations: u64,
    pub affinity_candidates: u64,
    pub cgroup_candidates: u64,
    /// `None` when any of the signals the guards need was not observed over the window.
    pub guards: Option<GuardFacts>,
}

impl AttemptResident {
    /// Events in the merged population.
    pub fn events(&self) -> u64 {
        self.histogram
            .iter()
            .fold(0u64, |total, bucket| total.saturating_add(bucket.count))
    }
}

#[derive(Debug)]
struct Running {
    target: ChildTarget,
    /// The controller's own rolling window type, fed the same events, so the guard facts are the production ones.
    window: RollingWindow,
    cgroup: String,
    attempt: Option<u32>,
    started_unix: u64,
    buckets: Vec<LatencyHistogramBucket>,
    max_ns: u64,
    lost: u64,
    invalid: u64,
    target_changed: bool,
    cpu: u64,
    gpu: u64,
    cpu_freq: u64,
    psi: u64,
    unattributed: u64,
    evaluations: u64,
    denied_evaluations: u64,
    affinity_candidates: u64,
    cgroup_candidates: u64,
}

#[derive(Debug, Default)]
pub struct ResidentObserver {
    running: Option<Running>,
    finished: Vec<AttemptResident>,
}

impl ResidentObserver {
    /// An attempt's game process is attested: start (or continue) observing it. A different process or attempt ends
    /// the previous window first.
    pub fn arm(&mut self, target: ChildTarget, attempt: Option<u32>, now_unix: u64) {
        if self
            .running
            .as_ref()
            .is_some_and(|running| running.target == target && running.attempt == attempt)
        {
            return;
        }
        self.end_attempt(now_unix);
        self.running = Some(Running {
            target,
            cgroup: cgroup_of(target.pid.as_u32()),
            window: RollingWindow::new(std::time::Duration::from_secs(24 * 3_600)),
            attempt,
            started_unix: now_unix,
            buckets: LatencyHistogram::new().snapshot(),
            max_ns: 0,
            lost: 0,
            invalid: 0,
            target_changed: false,
            cpu: 0,
            gpu: 0,
            cpu_freq: 0,
            psi: 0,
            unattributed: 0,
            evaluations: 0,
            denied_evaluations: 0,
            affinity_candidates: 0,
            cgroup_candidates: 0,
        });
    }

    /// An interval event: the records it carried and the events its sources dropped since the last one.
    pub fn observe_intervals(&mut self, records: &[IntervalRecord], lost_events: u64) {
        let Some(running) = self.running.as_mut() else {
            return;
        };
        running.lost = running.lost.saturating_add(lost_events);
        running.window.push_intervals(records.iter().cloned());
        let unattributed = records.iter().filter(|r| r.process_pid.is_none()).count();
        running.unattributed = running.unattributed.saturating_add(unattributed as u64);
        let mut seen = false;
        for record in records.iter().filter(|r| is_primary_game_class(r.class)) {
            match record.process_pid {
                Some(pid) if pid == running.target.pid.as_u32() => {}
                Some(_) => {
                    running.target_changed = true;
                    continue;
                }
                None => continue,
            }
            seen = true;
            let canonical = LatencyHistogram::new().snapshot();
            let count = record
                .histogram
                .iter()
                .try_fold(0u64, |total, bucket| total.checked_add(bucket.count));
            let valid = record.histogram.len() == canonical.len()
                && record
                    .histogram
                    .iter()
                    .zip(&canonical)
                    .all(|(a, b)| a.upper_bound_ns == b.upper_bound_ns)
                && count == Some(record.samples);
            if !valid {
                running.invalid = running.invalid.saturating_add(1);
                continue;
            }
            for (bucket, incoming) in running.buckets.iter_mut().zip(&record.histogram) {
                bucket.count = bucket.count.saturating_add(incoming.count);
            }
            running.max_ns = running.max_ns.max(record.max_ns);
        }
        if seen {
            // One scheduler and one pressure reading per interval in which the runtime saw the game.
            running.cpu = running.cpu.saturating_add(1);
            running.psi = running.psi.saturating_add(1);
        }
    }

    /// A planning decision the runtime just emitted about `target_root_pid`: one evaluation of the process, and which
    /// authorized actions it offered (eligible, or the one it selected).
    pub fn observe_decision(
        &mut self,
        target_root_pid: Option<u32>,
        planner: Option<&crate::autotune::planner::PlannerSummary>,
    ) {
        let Some(running) = self.running.as_mut() else {
            return;
        };
        if target_root_pid != Some(running.target.pid.as_u32()) {
            return;
        }
        // A decision without the planner's own summary is not an evaluation of the process, and neither is a pass the
        // planner skipped (observe mode, focus idle or unknown): none of them says whether an action was indicated.
        let Some(planner) = planner else {
            return;
        };
        let skipped = planner.no_action.as_ref().is_some_and(|none| {
            matches!(
                none.reason.as_str(),
                "observe mode does not suggest or apply" | "focus is idle or unknown"
            )
        });
        if skipped {
            return;
        }
        // A pass that offered nothing because every proposal was denied says the action was refused, not that it was
        // not indicated: it is kept apart and the abstention record refuses it.
        let denied = planner.selected.is_none()
            && planner.eligible_candidates.is_empty()
            && (!planner.grouped_denials.is_empty()
                || !planner.top_denied_candidates.is_empty()
                || !planner.missing_capabilities.is_empty()
                || !planner.workload_blocked.is_empty()
                || !planner.manual_only_suggestions.is_empty());
        if denied {
            running.denied_evaluations = running.denied_evaluations.saturating_add(1);
            return;
        }
        running.evaluations = running.evaluations.saturating_add(1);
        let offered = |kind: &str| {
            planner
                .eligible_candidates
                .iter()
                .any(|c| c.action_kind == kind)
                || planner
                    .selected
                    .as_ref()
                    .is_some_and(|s| s.action_kind == kind)
        };
        if offered("cpu_affinity_profile") {
            running.affinity_candidates = running.affinity_candidates.saturating_add(1);
        }
        if offered("cgroup_placement") {
            running.cgroup_candidates = running.cgroup_candidates.saturating_add(1);
        }
    }

    /// A GPU sample event while the attempt runs (counted only).
    pub fn observe_gpu_sample(&mut self) {
        if let Some(running) = self.running.as_mut() {
            running.gpu = running.gpu.saturating_add(1);
        }
    }

    /// A GPU sample event: counted, and held for the thermal and power guards.
    pub fn observe_gpu(&mut self, sample: &GpuSample) {
        self.observe_gpu_sample();
        if let Some(running) = self.running.as_mut() {
            running.window.push_gpu_sample(sample.clone());
        }
    }

    /// A frame event while the attempt runs, held for the foreground frame-tail guard.
    pub fn observe_frame(&mut self, frame: &FrameEvent) {
        if let Some(running) = self.running.as_mut() {
            running.window.push_frame(frame.clone());
        }
    }

    /// A CPU frequency event while the attempt runs, held for the power guard.
    pub fn observe_cpu_freq(&mut self, event: &CpuFreqRecord) {
        if let Some(running) = self.running.as_mut() {
            running.cpu_freq = running.cpu_freq.saturating_add(1);
            running.window.push_cpu_freq_event(event.clone());
        }
    }

    /// The attempt ended (or its process is gone): keep its window.
    pub fn end_attempt(&mut self, now_unix: u64) {
        if let Some(running) = self.running.take() {
            let members = std::collections::BTreeSet::from([running.target.pid.as_u32()]);
            let signals = running
                .window
                .objective_signals_for_target(running.target.pid.as_u32(), &members);
            let guards = (|| {
                Some(GuardFacts {
                    frame_p99_ms: signals.frame_p99_ms?,
                    frame_samples: signals.frame_samples?,
                    foreground_over_5ms: signals.foreground_over_5ms?,
                    thermal_degraded: signals.thermal_degraded?,
                    thermal_throttle_count: signals.thermal_throttle_count?,
                    cpu_power_limited: signals.cpu_power_limited?,
                    gpu_power_limited: signals.gpu_power_limited?,
                })
            })();
            self.finished.push(AttemptResident {
                target: running.target,
                cgroup: running.cgroup,
                attempt: running.attempt,
                started_unix: running.started_unix,
                // Timestamps are whole seconds: a window that opened and closed inside one second still spans that second.
                ended_unix: now_unix.max(running.started_unix.saturating_add(1)),
                histogram: running.buckets,
                max_ns: running.max_ns,
                lost_events: running.lost,
                invalid_intervals: running.invalid,
                target_changed: running.target_changed,
                cpu_samples: running.cpu,
                gpu_samples: running.gpu,
                cpu_freq_samples: running.cpu_freq,
                psi_samples: running.psi,
                unattributed_intervals: running.unattributed,
                evaluations: running.evaluations,
                denied_evaluations: running.denied_evaluations,
                affinity_candidates: running.affinity_candidates,
                cgroup_candidates: running.cgroup_candidates,
                guards,
            });
        }
    }

    /// Close the current window and copy every finished one so they can be persisted without holding the observer;
    /// `acknowledge` is then given that batch's length.
    pub fn freeze(&mut self, now_unix: u64) -> Vec<AttemptResident> {
        self.end_attempt(now_unix);
        self.finished.clone()
    }

    /// Drop exactly the first `count` windows, those a successful persistence covered.
    pub fn acknowledge(&mut self, count: usize) {
        let count = count.min(self.finished.len());
        self.finished.drain(..count);
    }

    /// The process the open window is armed on.
    #[cfg(test)]
    pub fn target(&self) -> Option<ChildTarget> {
        self.running.as_ref().map(|running| running.target)
    }

    pub fn is_observing(&self) -> bool {
        self.running.is_some()
    }
}

/// The unified-hierarchy cgroup of a process from its own `/proc/<pid>/cgroup`, empty when it cannot be read.
fn cgroup_of(pid: u32) -> String {
    std::fs::read_to_string(format!("/proc/{pid}/cgroup"))
        .ok()
        .and_then(|text| {
            text.lines()
                .find_map(|line| line.strip_prefix("0::").map(str::to_owned))
        })
        .unwrap_or_default()
}

/// The file, beside a study, holding every attempt's raw observed window.
pub const WINDOWS_FILE: &str = "resident-windows.json";

/// One window as raw counters, the immutable evidence the sealed records are derived from.
pub fn window_record(window: &AttemptResident) -> serde_json::Value {
    serde_json::json!({
        "attempt": window.attempt, "cgroup": window.cgroup,
        "pid": window.target.pid, "start_ticks": window.target.start_ticks, "uid": window.target.uid,
        "started_unix": window.started_unix, "ended_unix": window.ended_unix,
        "histogram": window.histogram, "max_ns": window.max_ns,
        "lost_events": window.lost_events, "invalid_intervals": window.invalid_intervals,
        "target_changed": window.target_changed,
        "cpu_samples": window.cpu_samples, "gpu_samples": window.gpu_samples, "cpu_freq_samples": window.cpu_freq_samples,
        "psi_samples": window.psi_samples, "unattributed_intervals": window.unattributed_intervals, "evaluations": window.evaluations,
        "denied_evaluations": window.denied_evaluations,
        "affinity_candidates": window.affinity_candidates, "cgroup_candidates": window.cgroup_candidates,
        "guards": window.guards.map(|g| serde_json::json!({
            "frame_p99_ms": g.frame_p99_ms, "frame_samples": g.frame_samples,
            "foreground_over_5ms": g.foreground_over_5ms, "thermal_degraded": g.thermal_degraded,
            "thermal_throttle_count": g.thermal_throttle_count, "cpu_power_limited": g.cpu_power_limited,
            "gpu_power_limited": g.gpu_power_limited,
        })),
    })
}

/// The resident-observation entry of one finished window, from what was counted (nothing is stated): the lost samples
/// are the dropped events and the unusable intervals, and a window that saw another game process is lossy too.
pub fn resident_entry(
    window: &AttemptResident,
    process: &AttemptProcess,
    cgroup: &str,
    renderer_bound: bool,
    owners_identified: bool,
    runnable_claim: bool,
) -> serde_json::Value {
    let mut collector =
        super::evidence_producers::ResidentCollector::new(process.clone(), window.started_unix);
    collector.renderer_bound(renderer_bound);
    collector.cgroup(cgroup);
    collector.owners_identified(owners_identified);
    for _ in 0..window.cpu_samples {
        collector.cpu_sample();
    }
    for _ in 0..window.gpu_samples {
        collector.gpu_sample();
    }
    for _ in 0..window.psi_samples {
        collector.psi_sample();
    }
    let lost = window
        .lost_events
        .saturating_add(window.invalid_intervals)
        .saturating_add(u64::from(window.target_changed));
    collector.lost_samples(lost);
    collector.runnable(window.events(), lost);
    if runnable_claim {
        collector.claim_runnable();
    }
    collector.finish(window.ended_unix)
}

#[cfg(test)]
#[path = "resident_observer_tests.rs"]
pub(crate) mod tests;
