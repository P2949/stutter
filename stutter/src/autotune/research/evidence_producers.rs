//! The producer half of the sealed records the independent verifiers read: collectors the resident runtime feeds
//! during an attempt and that turn what it counted into exactly the documents `resident_observation` and
//! `action_latency` verify. Nothing here measures anything: a collector holds only what it was given, derives the
//! histogram and the counts from raw samples (never a stated percentile or a declared quality), and the output is
//! checked by the same verifiers a campaign is, so producer and verifier cannot drift apart. Feeding a collector
//! from the live game needs the game; the wiring into the runtime's event path is the one thing not done here.

use serde_json::{Value, json};

use super::child_verify::AttemptProcess;

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

/// What the foreground and the host reported over a window, for the production guardrails.
#[derive(Clone, Copy, Debug)]
pub struct WindowGuards {
    pub foreground_p99_ms: f64,
    pub foreground_samples: u64,
    pub foreground_over_5ms: u64,
    pub thermal_degraded: bool,
    pub thermal_throttle_count: u64,
    pub cpu_power_limited: bool,
    pub gpu_power_limited: bool,
}

/// One attempt's runnable-delay population, counted event by event into the controller's own canonical histogram
/// (the same buckets, so a quantization the controller has, the replay has, and no other).
#[derive(Clone, Debug)]
pub struct LatencyWindowBuilder {
    start_unix: u64,
    buckets: Vec<crate::metrics::LatencyHistogramBucket>,
    max_ns: u64,
    lost: u64,
    same_target: bool,
}

impl LatencyWindowBuilder {
    pub fn new(now_unix: u64) -> Self {
        Self {
            start_unix: now_unix,
            buckets: crate::metrics::LatencyHistogram::new().snapshot(),
            max_ns: 0,
            lost: 0,
            same_target: true,
        }
    }
    /// One runnable-delay event of the scoped game target, in nanoseconds as the kernel source reports it. A delay
    /// that is not a number of nanoseconds cannot be passed: the type is the validation.
    pub fn event(&mut self, delay_ns: u64) {
        // The controller's own rule: the first bucket whose bound holds the delay, else the unbounded last one.
        let index = self
            .buckets
            .iter()
            .position(|bucket| bucket.upper_bound_ns.is_some_and(|bound| delay_ns <= bound))
            .unwrap_or(self.buckets.len() - 1);
        self.buckets[index].count = self.buckets[index].count.saturating_add(1);
        self.max_ns = self.max_ns.max(delay_ns);
    }

    /// A population the resident runtime already merged in the controller's canonical layout (its interval records'
    /// histograms summed). A layout that is not the canonical one is refused, never reinterpreted.
    pub fn from_population(
        now_unix: u64,
        buckets: Vec<crate::metrics::LatencyHistogramBucket>,
        max_ns: u64,
    ) -> Option<Self> {
        let canonical = crate::metrics::LatencyHistogram::new().snapshot();
        let same_layout = buckets.len() == canonical.len()
            && buckets
                .iter()
                .zip(&canonical)
                .all(|(a, b)| a.upper_bound_ns == b.upper_bound_ns);
        same_layout.then_some(Self {
            start_unix: now_unix,
            buckets,
            max_ns,
            lost: 0,
            same_target: true,
        })
    }
    /// Events the source dropped, or samples that could not be read: the window is then not a population.
    pub fn lost(&mut self, count: u64) {
        self.lost = self.lost.saturating_add(count);
    }
    /// The scoped target changed during the window (a different process): the window is not one population.
    pub fn target_changed(&mut self) {
        self.same_target = false;
    }
    /// The window ended at `end_unix`, with the guard facts the host observed over the same time.
    pub fn finish(&self, end_unix: u64, guards: WindowGuards) -> Value {
        json!({
            "start_unix": self.start_unix, "end_unix": end_unix, "lost": self.lost,
            "same_target": self.same_target, "histogram": self.buckets,
            "max_ns": self.max_ns,
            "foreground_p99_ms": guards.foreground_p99_ms,
            "foreground_samples": guards.foreground_samples,
            "foreground_over_5ms": guards.foreground_over_5ms,
            "thermal_degraded": guards.thermal_degraded,
            "thermal_throttle_count": guards.thermal_throttle_count,
            "cpu_power_limited": guards.cpu_power_limited,
            "gpu_power_limited": guards.gpu_power_limited,
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
    serde_json::to_vec(&json!({ "schema": 3, "metric": "GameRunnableLatency", "pairs": pairs }))
        .unwrap_or_default()
}

/// One boundary decision of the privileged worker in its audit log's format: an allowed, successful `operation` (apply or
/// rollback) of `action_id` that touched `tasks` tasks, at `at_unix`.
pub fn worker_audit_event(action_id: &str, operation: &str, at_unix: u64, tasks: usize) -> Value {
    json!({
        "schema_version": 1, "unix_nanos": u128::from(at_unix) * 1_000_000_000, "command": "daemon_privilege",
        "action_id": action_id, "safety_class": "ReversibleLowRisk", "dry_run": false, "success": true,
        "affected_tasks": tasks, "restore_path": null,
        "message": format!("stage=execution verdict=allow reason_code=ok caller_role=privileged_worker transport=socket operation={operation} policy_intent=Apply request=apply action_id={action_id} error_category=ok detail=ok"),
    })
}

/// One transaction's final record in the controller's own journal format (applied, verified, rolled back), as the
/// resident runtime archives it beside the study.
pub fn controller_journal_record(
    transaction: &str,
    action_id: &str,
    process: (u32, u64),
    window_unix: (u64, u64),
) -> Value {
    json!({
        "boot_id": "boot", "schema_version": 1, "state": "reverted",
        "experiment_id": transaction, "action_id": action_id,
        "target_identity": format!("pid:{}:starttime:{}:active_tasks:12", process.0, process.1),
        "verify_result": "rollback_verified",
        "rollback_token": { "kind": "cpu-affinity-restore-file", "restore_path": "restore.json", "affected_tasks": 12 },
        "phase_started_unix_nanos": u128::from(window_unix.0) * 1_000_000_000,
        "updated_unix_nanos": u128::from(window_unix.1) * 1_000_000_000,
    })
}

#[cfg(test)]
#[path = "evidence_producers_tests.rs"]
mod tests;
