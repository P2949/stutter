//! The producer half of the sealed records the independent verifiers read: collectors the resident runtime feeds
//! during an attempt and that turn what it counted into exactly the documents `resident_observation` and
//! `action_latency` verify. Nothing here measures anything: a collector holds only what it was given, derives the
//! histogram and the counts from raw samples (never a stated percentile or a declared quality), and the output is
//! checked by the same verifiers a campaign is, so producer and verifier cannot drift apart. Feeding a collector
//! from the live game needs the game; the wiring into the runtime's event path is the one thing not done here.

use serde_json::{Value, json};

use super::child_verify::AttemptProcess;

/// Histogram bounds in milliseconds, ascending; the last bucket takes everything above the previous bound.
const BOUNDS_MS: [f64; 12] = [
    0.05,
    0.1,
    0.25,
    0.5,
    1.0,
    2.0,
    4.0,
    8.0,
    16.0,
    32.0,
    64.0,
    1_000_000.0,
];

/// What one attempt's resident observation is built from.
#[derive(Clone, Debug)]
pub struct ResidentCollector {
    process: AttemptProcess,
    window_start: u64,
    renderer_bound: bool,
    cgroup: String,
    cpu: u64,
    gpu: u64,
    psi: u64,
    lost: u64,
    owners_identified: bool,
    runnable_events: u64,
    runnable_lost: u64,
    runnable_claim: bool,
}

impl ResidentCollector {
    /// Start observing the attested game process of one attempt at `now_unix`.
    pub fn new(process: AttemptProcess, now_unix: u64) -> Self {
        Self {
            process,
            window_start: now_unix,
            renderer_bound: false,
            cgroup: String::new(),
            cpu: 0,
            gpu: 0,
            psi: 0,
            lost: 0,
            owners_identified: false,
            runnable_events: 0,
            runnable_lost: 0,
            runnable_claim: false,
        }
    }
    pub fn renderer_bound(&mut self, bound: bool) {
        self.renderer_bound = bound;
    }
    pub fn cgroup(&mut self, path: &str) {
        self.cgroup = path.to_owned();
    }
    pub fn cpu_sample(&mut self) {
        self.cpu += 1;
    }
    pub fn gpu_sample(&mut self) {
        self.gpu += 1;
    }
    pub fn psi_sample(&mut self) {
        self.psi += 1;
    }
    pub fn lost_samples(&mut self, count: u64) {
        self.lost += count;
    }
    pub fn owners_identified(&mut self, identified: bool) {
        self.owners_identified = identified;
    }
    pub fn runnable(&mut self, events: u64, lost: u64) {
        self.runnable_events += events;
        self.runnable_lost += lost;
    }
    /// This attempt will be used to claim a runnable-latency measurement.
    pub fn claim_runnable(&mut self) {
        self.runnable_claim = true;
    }
    /// The attempt ended at `end_unix`: the entry for the record.
    pub fn finish(&self, end_unix: u64) -> Value {
        json!({
            "slot": self.process.slot, "pid": self.process.pid, "start_ticks": self.process.start_ticks,
            "uid": self.process.uid, "window_start_unix": self.window_start, "window_end_unix": end_unix,
            "renderer_bound": self.renderer_bound, "cgroup": self.cgroup,
            "cpu_samples": self.cpu, "gpu_samples": self.gpu, "psi_samples": self.psi,
            "lost_samples": self.lost, "owners_identified": self.owners_identified,
            "runnable_events": self.runnable_events, "runnable_lost": self.runnable_lost,
            "runnable_claim": self.runnable_claim,
        })
    }
}

/// The document `resident_observation` verifies, from every attempt's finished entry.
pub fn resident_observation_document(entries: Vec<Value>) -> Vec<u8> {
    serde_json::to_vec(&json!({ "schema": 2, "attempts": entries })).unwrap_or_default()
}

/// One attempt's runnable-delay population, counted event by event.
#[derive(Clone, Debug)]
pub struct LatencyWindowBuilder {
    start_unix: u64,
    counts: [u64; BOUNDS_MS.len()],
    lost: u64,
    same_target: bool,
}

impl LatencyWindowBuilder {
    pub fn new(now_unix: u64) -> Self {
        Self {
            start_unix: now_unix,
            counts: [0; BOUNDS_MS.len()],
            lost: 0,
            same_target: true,
        }
    }
    /// One runnable-delay event of the scoped game target, in milliseconds.
    pub fn event(&mut self, delay_ms: f64) {
        let index = BOUNDS_MS
            .iter()
            .position(|bound| delay_ms <= *bound)
            .unwrap_or(BOUNDS_MS.len() - 1);
        self.counts[index] += 1;
    }
    pub fn lost(&mut self, count: u64) {
        self.lost += count;
    }
    /// The scoped target changed during the window (a different process): the window is not one population.
    pub fn target_changed(&mut self) {
        self.same_target = false;
    }
    /// The window ended at `end_unix`, with the foreground's own tail over the same time.
    pub fn finish(&self, end_unix: u64, foreground_p99_ms: f64) -> Value {
        let histogram: Vec<Value> = BOUNDS_MS
            .iter()
            .zip(&self.counts)
            .filter(|(_, count)| **count > 0)
            .map(|(bound, count)| json!({ "le_ms": bound, "count": count }))
            .collect();
        json!({
            "start_unix": self.start_unix, "end_unix": end_unix, "lost": self.lost,
            "same_target": self.same_target, "histogram": histogram,
            "foreground_p99_ms": foreground_p99_ms,
        })
    }
}

/// One planned pair of an action study: the Off and On attempts with the very processes they recorded.
pub fn latency_pair(off: (&AttemptProcess, Value), on: (&AttemptProcess, Value)) -> Value {
    let identity =
        |p: &AttemptProcess| json!({ "pid": p.pid, "start_ticks": p.start_ticks, "uid": p.uid });
    json!({
        "off_slot": off.0.slot, "on_slot": on.0.slot,
        "off_process": identity(off.0), "on_process": identity(on.0),
        "off": off.1, "on": on.1,
    })
}

/// The document `action_latency` verifies.
pub fn action_latency_document(pairs: Vec<Value>) -> Vec<u8> {
    serde_json::to_vec(&json!({ "schema": 2, "metric": "GameRunnableLatency", "pairs": pairs }))
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "evidence_producers_tests.rs"]
mod tests;
