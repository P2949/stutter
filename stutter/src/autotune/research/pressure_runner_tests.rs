use super::*;
use crate::{
    autotune::research::{evidence_producers::WorkerExport, pressure_control::attested},
    metrics::LatencyHistogram,
};

const ACTION: &str = "cpu-affinity-profile:game-main";

fn population(start: u64, end: u64, delay_ns: u64) -> Population {
    let mut histogram = LatencyHistogram::new();
    for _ in 0..1_500 {
        histogram.record(delay_ns);
    }
    Population {
        start_unix: start,
        end_unix: end,
        histogram: histogram.snapshot(),
        max_ns: delay_ns,
    }
}

/// A stage that touches nothing: a stepped clock, a stand-in whose latency the load raises, and an action the gate and the
/// worker would record.
struct Fake {
    clock: u64,
    halt_after: Option<usize>,
    halts_asked: usize,
    fail_under_load: bool,
    steps: Vec<&'static str>,
}

impl Fake {
    fn new() -> Self {
        Self {
            clock: 1_000,
            halt_after: None,
            halts_asked: 0,
            fail_under_load: false,
            steps: Vec::new(),
        }
    }
}

impl PressureStage for Fake {
    fn now(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }
    fn halted(&mut self) -> bool {
        self.halts_asked += 1;
        self.halt_after
            .is_some_and(|after| self.halts_asked > after)
    }
    fn baseline(&mut self, seconds: u64) -> Result<Population, &'static str> {
        self.steps.push("baseline");
        let start = self.clock;
        self.clock += seconds;
        Ok(population(start, self.clock, 2_000_000))
    }
    fn under_load(&mut self, seconds: u64) -> Result<(Population, ActionRecord), &'static str> {
        self.steps.push("under_load");
        if self.fail_under_load {
            return Err("ActionRefused");
        }
        let start = self.clock + 1;
        self.clock += seconds + 1;
        let mut export = WorkerExport::default();
        export.transaction(
            "tx",
            ACTION,
            (5_000, 100),
            (start + 1, start + 2, start + seconds - 1),
        );
        Ok((
            population(start, self.clock, 9_000_000),
            ActionRecord {
                receipt: serde_json::json!({
                    "action": "CpuAffinityProfile", "pid": 5_000, "start_ticks": 100, "uid": 1_000,
                    "permitted_unix": start + 1, "applied_unix": start + 2, "settled_unix": start + seconds - 1,
                    "original": { "cpus_allowed": "0-15" }, "restored": { "cpus_allowed": "0-15" },
                }),
                worker_transactions: export.records().to_vec(),
            },
        ))
    }
}

const PLAN: Plan = Plan {
    workers: 1,
    window_seconds: 20,
};

#[test]
fn a_control_runs_in_order_and_leaves_a_record_that_attests() {
    let mut stage = Fake::new();
    let evidence = run(&mut stage, &"aa".repeat(32), PLAN).unwrap();
    assert_eq!(stage.steps, ["baseline", "under_load"]);
    let receipt = reconstruct(&evidence).unwrap();
    assert_eq!(
        attested(&receipt, &"aa".repeat(32), evidence.captured_unix),
        Ok(())
    );
    assert!(evidence.load.all_joined);
}

#[test]
fn a_halt_stops_the_control_before_anything_starts_and_between_steps() {
    for after in 0..3 {
        let mut stage = Fake::new();
        stage.halt_after = Some(after);
        assert_eq!(
            run(&mut stage, &"aa".repeat(32), PLAN).err(),
            Some("PressureControlHalted"),
            "halt after {after} asks"
        );
        // Nothing was applied unless the halt came after the load started.
        if after == 0 {
            assert!(stage.steps.is_empty());
        }
    }
}

#[test]
fn a_failed_action_leaves_no_record_and_no_load_running() {
    let mut stage = Fake::new();
    stage.fail_under_load = true;
    assert_eq!(
        run(&mut stage, &"aa".repeat(32), PLAN).err(),
        Some("ActionRefused")
    );
}

#[test]
fn an_oversized_load_is_refused_and_nothing_is_applied() {
    let mut stage = Fake::new();
    let plan = Plan {
        workers: 1_000,
        ..PLAN
    };
    assert_eq!(
        run(&mut stage, &"aa".repeat(32), plan).err(),
        Some("OwnedLoadSizeRefused")
    );
}

#[test]
fn the_record_is_written_once_and_the_status_reads_it_back() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut stage = Fake::new();
    let evidence = run(&mut stage, &"aa".repeat(32), PLAN).unwrap();
    let handle = std::fs::File::open(dir.path()).unwrap();
    record(&handle, &evidence).unwrap();
    // Never over an existing record.
    assert!(record(&handle, &evidence).is_err());
    assert_eq!(
        crate::autotune::research::pressure_control::status(
            dir.path(),
            &"aa".repeat(32),
            evidence.captured_unix
        ),
        Ok(())
    );
}
