//! The resident runtime's own observation of a research attempt: the shared observer the funnel arms on the attested
//! game process and the runtime feeds from the events it already handles (interval records, GPU samples). It counts
//! only what the runtime reports of that exact process, in the controller's canonical histogram layout, and keeps one
//! finished window per attempt for the sealed records the independent verifiers read. It never decides anything and
//! never reaches the controller: a second sensor path or a second optimizer is not created, the existing events are
//! read.

use std::sync::{Arc, Mutex};

use super::{action_child::ChildTarget, child_verify::AttemptProcess};
use crate::{
    autotune::objective::runnable_signal::is_primary_game_class,
    metrics::{IntervalRecord, LatencyHistogram, LatencyHistogramBucket},
};

/// Shared between the runtime (which reports its events) and the funnel (which opens and closes attempts).
pub type SharedResident = Arc<Mutex<ResidentObserver>>;

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
    pub psi_samples: u64,
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
    psi: u64,
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
            attempt,
            started_unix: now_unix,
            buckets: LatencyHistogram::new().snapshot(),
            max_ns: 0,
            lost: 0,
            invalid: 0,
            target_changed: false,
            cpu: 0,
            gpu: 0,
            psi: 0,
        });
    }

    /// An interval event: the records it carried and the events its sources dropped since the last one.
    pub fn observe_intervals(&mut self, records: &[IntervalRecord], lost_events: u64) {
        let Some(running) = self.running.as_mut() else {
            return;
        };
        running.lost = running.lost.saturating_add(lost_events);
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

    /// A GPU sample event while the attempt runs.
    pub fn observe_gpu_sample(&mut self) {
        if let Some(running) = self.running.as_mut() {
            running.gpu = running.gpu.saturating_add(1);
        }
    }

    /// The attempt ended (or its process is gone): keep its window.
    pub fn end_attempt(&mut self, now_unix: u64) {
        if let Some(running) = self.running.take() {
            self.finished.push(AttemptResident {
                target: running.target,
                cgroup: running.cgroup,
                attempt: running.attempt,
                started_unix: running.started_unix,
                ended_unix: now_unix,
                histogram: running.buckets,
                max_ns: running.max_ns,
                lost_events: running.lost,
                invalid_intervals: running.invalid,
                target_changed: running.target_changed,
                cpu_samples: running.cpu,
                gpu_samples: running.gpu,
                psi_samples: running.psi,
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
        "cpu_samples": window.cpu_samples, "gpu_samples": window.gpu_samples,
        "psi_samples": window.psi_samples,
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
mod tests;
