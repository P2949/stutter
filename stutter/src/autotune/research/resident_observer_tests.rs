use super::*;
use crate::{
    autotune::research::{child_verify::AttemptProcess, resident_observation},
    process_tree::TaskClass,
};

fn target() -> ChildTarget {
    ChildTarget {
        pid: stutter_core::ids::Pid::new(4242),
        start_ticks: 99,
        uid: 1000,
    }
}

/// An interval record of `pid`'s game threads holding `events` delays of `delay_ns`.
fn interval(pid: u32, class: TaskClass, events: u64, delay_ns: u64) -> IntervalRecord {
    let mut histogram = LatencyHistogram::new();
    for _ in 0..events {
        histogram.record(delay_ns);
    }
    let snapshot = histogram.snapshot();
    IntervalRecord {
        process_pid: Some(pid),
        class,
        samples: events,
        max_ns: delay_ns,
        histogram: snapshot,
        ..Default::default()
    }
}

fn process() -> AttemptProcess {
    AttemptProcess {
        slot: 0,
        pid: target().pid,
        start_ticks: 99,
        uid: 1000,
        started_unix: 1_000,
        completed_unix: 1_100,
    }
}

fn observe(window_edit: impl FnOnce(&mut ResidentObserver)) -> AttemptResident {
    let mut observer = ResidentObserver::default();
    observer.arm(target(), Some(0), 1_010);
    for _ in 0..12 {
        observer.observe_intervals(
            &[
                interval(4242, TaskClass::Game, 100, 500_000),
                interval(4242, TaskClass::GameRenderThread, 100, 8_000_000),
                interval(9, TaskClass::Compiler, 5_000, 90_000_000),
            ],
            0,
        );
        observer.observe_gpu_sample();
    }
    window_edit(&mut observer);
    observer.freeze(1_090).remove(0)
}

fn verified(window: &AttemptResident, claim: bool) -> Result<(), &'static str> {
    let entry = resident_entry(window, &process(), "0::/app.slice", true, true, claim);
    let document =
        crate::autotune::research::evidence_producers::resident_observation_document(vec![entry]);
    resident_observation::verify(&document, &[process()])
}

#[test]
fn only_the_attested_games_population_is_counted_from_the_runtimes_own_events() {
    let window = observe(|_| {});
    // 12 intervals x (100 + 100) game events; the compiler's and everything else's are not the game's.
    assert_eq!(window.events(), 2_400);
    assert_eq!(
        (window.cpu_samples, window.gpu_samples, window.psi_samples),
        (12, 12, 12)
    );
    assert_eq!(
        (
            window.lost_events,
            window.invalid_intervals,
            window.target_changed
        ),
        (0, 0, false)
    );
    assert_eq!(window.max_ns, 8_000_000);
    // What it counted verifies, as an observation and as a runnable-latency claim.
    assert_eq!(verified(&window, false), Ok(()));
    assert_eq!(verified(&window, true), Ok(()));
}

#[test]
fn drops_invalid_intervals_a_second_game_process_and_a_thin_window_each_stop_it_being_an_observation()
 {
    // The sources dropped events: the window is lossy, and a runnable claim is refused.
    let lossy = observe(|o| o.observe_intervals(&[], 3));
    assert_eq!(lossy.lost_events, 3);
    assert!(verified(&lossy, true).is_err());
    // An interval of the target that is not a canonical population is counted as unusable, never merged.
    let mut bad = interval(4242, TaskClass::Game, 10, 1_000_000);
    bad.histogram.pop();
    let invalid = observe(|o| o.observe_intervals(&[bad], 0));
    assert_eq!(invalid.invalid_intervals, 1);
    assert!(verified(&invalid, true).is_err());
    // Another game process in the target's intervals: the window is not one population.
    let moved =
        observe(|o| o.observe_intervals(&[interval(777, TaskClass::Game, 100, 500_000)], 0));
    assert!(moved.target_changed);
    assert!(verified(&moved, true).is_err());
    // Too few samples of any kind is not enough to say the game was watched.
    let mut thin = ResidentObserver::default();
    thin.arm(target(), Some(0), 1_010);
    thin.observe_intervals(&[interval(4242, TaskClass::Game, 100, 500_000)], 0);
    let window = thin.freeze(1_090).remove(0);
    assert!(verified(&window, false).is_err());
}

#[test]
fn a_window_belongs_to_one_attempt_and_one_process_and_persisted_windows_are_acknowledged_exactly()
{
    let mut observer = ResidentObserver::default();
    observer.arm(target(), Some(0), 1_000);
    observer.observe_intervals(&[interval(4242, TaskClass::Game, 50, 500_000)], 0);
    // The same attempt and process: it continues. Another process: the first window ends and a new one begins.
    observer.arm(target(), Some(0), 1_001);
    let other = ChildTarget {
        pid: stutter_core::ids::Pid::new(5000),
        ..target()
    };
    observer.arm(other, Some(1), 1_050);
    observer.observe_intervals(&[interval(5000, TaskClass::Game, 70, 500_000)], 0);
    let frozen = observer.freeze(1_100);
    assert_eq!(frozen.len(), 2);
    assert_eq!((frozen[0].events(), frozen[1].events()), (50, 70));
    assert!(!observer.is_observing());
    // Only the windows the persistence covered are dropped; one finished since the freeze is kept.
    observer.arm(target(), Some(2), 1_200);
    observer.end_attempt(1_300);
    observer.acknowledge(frozen.len());
    assert_eq!(observer.freeze(1_300).len(), 1);
    // Nothing is observed when nothing is armed.
    let mut idle = ResidentObserver::default();
    idle.observe_intervals(&[interval(4242, TaskClass::Game, 50, 500_000)], 0);
    idle.observe_gpu_sample();
    assert!(idle.freeze(1).is_empty());
}

#[test]
fn an_observed_window_becomes_a_latency_window_only_in_the_canonical_layout() {
    use crate::autotune::research::evidence_producers::{LatencyWindowBuilder, WindowGuards};
    let window = observe(|_| {});
    let guards = WindowGuards {
        foreground_p99_ms: 12.0,
        foreground_samples: 1_000,
        foreground_over_5ms: 10,
        thermal_degraded: false,
        thermal_throttle_count: 0,
        cpu_power_limited: false,
        gpu_power_limited: false,
    };
    let built = LatencyWindowBuilder::from_population(
        window.started_unix,
        window.histogram.clone(),
        window.max_ns,
    )
    .expect("the observer's own population is canonical")
    .finish(window.ended_unix, guards);
    assert_eq!(built["max_ns"], 8_000_000);
    let total: u64 = built["histogram"]
        .as_array()
        .unwrap()
        .iter()
        .map(|bucket| bucket["count"].as_u64().unwrap())
        .sum();
    assert_eq!(total, window.events());
    // Another layout is refused, never reinterpreted.
    let mut other = window.histogram.clone();
    other.pop();
    assert!(LatencyWindowBuilder::from_population(1, other, 1).is_none());
}
