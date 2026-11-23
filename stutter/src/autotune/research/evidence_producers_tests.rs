use super::*;
use crate::autotune::research::{Treatment, action_latency, resident_observation};

fn process(slot: u32) -> AttemptProcess {
    AttemptProcess {
        slot,
        pid: stutter_core::ids::Pid::new(100 + slot),
        start_ticks: 50 + u64::from(slot),
        uid: 1000,
        started_unix: 1_000,
        completed_unix: 1_100,
    }
}

#[test]
fn what_the_collector_counts_is_what_the_resident_verifier_accepts_and_refuses() {
    let processes: Vec<_> = (0..2).map(process).collect();
    let entry = |p: &AttemptProcess, edit: &dyn Fn(&mut ResidentCollector)| {
        let mut c = ResidentCollector::new(p.clone(), 1_005);
        c.renderer_bound(true);
        c.cgroup("0::/app.slice/game");
        c.owners_identified(true);
        for _ in 0..20 {
            c.cpu_sample();
            c.gpu_sample();
            c.psi_sample();
        }
        c.runnable(12, 0);
        edit(&mut c);
        c.finish(1_095)
    };
    let good = resident_observation_document(processes.iter().map(|p| entry(p, &|_| {})).collect());
    assert_eq!(resident_observation::verify(&good, &processes), Ok(()));
    // A runnable claim with too few events, a lossy source and a half-bound attempt are each caught.
    let claimed = resident_observation_document(vec![
        entry(&processes[0], &|c| c.claim_runnable()),
        entry(&processes[1], &|_| {}),
    ]);
    assert_eq!(
        resident_observation::verify(&claimed, &processes),
        Err("ResidentObservationRunnableFloor")
    );
    let lossy = resident_observation_document(vec![
        entry(&processes[0], &|c| c.lost_samples(1)),
        entry(&processes[1], &|_| {}),
    ]);
    assert_eq!(
        resident_observation::verify(&lossy, &processes),
        Err("ResidentObservationSourceLossy")
    );
    let unbound = resident_observation_document(vec![
        entry(&processes[0], &|c| c.renderer_bound(false)),
        entry(&processes[1], &|_| {}),
    ]);
    assert_eq!(
        resident_observation::verify(&unbound, &processes),
        Err("ResidentObservationIncomplete")
    );
}

const MS: u64 = 1_000_000;

fn guards(foreground: f64) -> WindowGuards {
    WindowGuards {
        foreground_p99_ms: foreground,
        foreground_samples: 1_000,
        foreground_over_5ms: 10,
        thermal_degraded: false,
        thermal_throttle_count: 0,
        cpu_power_limited: false,
        gpu_power_limited: false,
    }
}

/// A window of 2000 events: 95% fast and 5% at `tail_ns`, so the 99th percentile sits in the tail.
fn window(tail_ns: u64, foreground: f64) -> Value {
    let mut w = LatencyWindowBuilder::new(1_010);
    for _ in 0..1_900 {
        w.event(500_000);
    }
    for _ in 0..100 {
        w.event(tail_ns);
    }
    w.finish(1_090, guards(foreground))
}

#[test]
fn the_latency_collector_builds_the_populations_the_objective_verifier_recomputes_from() {
    let processes: Vec<_> = (0..8).map(process).collect();
    use Treatment::{Off, On};
    let schedule = vec![Off, On, On, Off, Off, On, On, Off];
    let pairs = |off_tail: u64, on_tail: u64| {
        action_latency_document(
            [(0, 1), (3, 2), (4, 5), (7, 6)]
                .into_iter()
                .map(|(off, on)| {
                    latency_pair(
                        (&processes[off], window(off_tail, 12.0)),
                        (&processes[on], window(on_tail, 12.0)),
                    )
                })
                .collect(),
        )
    };
    let result =
        action_latency::verify(&pairs(20 * MS, 3 * MS / 2), &schedule, &processes).unwrap();
    assert_eq!(
        (result.pairs, result.decision),
        (4, action_latency::Decision::Keep)
    );
    let worse = action_latency::verify(&pairs(3 * MS / 2, 20 * MS), &schedule, &processes).unwrap();
    assert_eq!(worse.decision, action_latency::Decision::Revert);
    // Too few events, a lost event and a changed target each stop it being a measurement.
    let mut thin = LatencyWindowBuilder::new(1_010);
    for _ in 0..10 {
        thin.event(MS);
    }
    let thin_doc = action_latency_document(vec![latency_pair(
        (&processes[0], thin.finish(1_090, guards(12.0))),
        (&processes[1], window(3 * MS / 2, 12.0)),
    )]);
    assert!(action_latency::verify(&thin_doc, &schedule, &processes).is_err());
    let mut lossy = LatencyWindowBuilder::new(1_010);
    for _ in 0..2_000 {
        lossy.event(MS);
    }
    lossy.lost(1);
    assert_eq!(lossy.finish(1_090, guards(12.0))["lost"], 1);
    let mut moved = LatencyWindowBuilder::new(1_010);
    moved.target_changed();
    assert_eq!(moved.finish(1_090, guards(12.0))["same_target"], false);
}
