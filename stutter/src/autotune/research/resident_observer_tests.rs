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

#[test]
fn the_cgroup_is_read_from_the_kernels_view_of_the_attested_process() {
    let own = ChildTarget {
        pid: stutter_core::ids::Pid::new(std::process::id()),
        start_ticks: 1,
        uid: 1000,
    };
    let mut observer = ResidentObserver::default();
    observer.arm(own, Some(0), 10);
    let window = observer.freeze(20).remove(0);
    assert!(window.cgroup.starts_with('/'), "{:?}", window.cgroup);
    // A process that is not there shows none, and an observation then cannot claim one.
    observer.arm(
        ChildTarget {
            pid: stutter_core::ids::Pid::new(u32::MAX - 1),
            ..own
        },
        Some(1),
        30,
    );
    assert!(observer.freeze(40).remove(1).cgroup.is_empty());
}

pub(crate) fn offering(kind: &str) -> crate::autotune::planner::PlannerSummary {
    crate::autotune::planner::PlannerSummary {
        eligible_candidates: vec![crate::autotune::planner::PlannerEvaluationSummary {
            candidate_name: "game-main".to_owned(),
            action_kind: kind.to_owned(),
            provider: "profile".to_owned(),
            objective: crate::autotune::objective::ObjectiveKind::StutterScore,
            safety_class: crate::actions::SafetyClass::ReversibleLowRisk,
            effect_scope: crate::daemon_policy::ActionEffectScope::LocalProcessTree,
            confidence: 0.9,
            eligible: true,
            rank: Some(1),
            deny_reasons: Vec::new(),
            deny_reason_codes: Vec::new(),
            deny_messages: Vec::new(),
            dry_run_affected_tasks: Some(3),
            manual_only_reason: None,
            evidence: Vec::new(),
        }],
        ..Default::default()
    }
}

#[test]
fn the_planners_decisions_about_the_attested_process_are_counted_with_the_authorized_actions_it_offered()
 {
    let mut observer = ResidentObserver::default();
    // Before anything is attested, and about another process, nothing is counted.
    observer.observe_decision(Some(4242), None);
    observer.arm(target(), Some(0), 1_000);
    observer.observe_decision(Some(9), Some(&offering("cpu_affinity_profile")));
    // A decision without the planner's own summary is not an evaluation of the process.
    observer.observe_decision(Some(4242), None);
    observer.observe_decision(Some(4242), Some(&Default::default()));
    observer.observe_decision(Some(4242), Some(&offering("cpu_affinity_profile")));
    observer.observe_decision(Some(4242), Some(&offering("cgroup_placement")));
    observer.observe_decision(Some(4242), Some(&offering("nice_priority")));
    let window = observer.freeze(1_100).remove(0);
    assert_eq!(
        (
            window.evaluations,
            window.affinity_candidates,
            window.cgroup_candidates
        ),
        (4, 1, 1)
    );
}

/// Only a pass the planner really ran and that was not simply refused counts toward `ActionNotIndicated`: a skipped
/// pass (observe mode, focus idle) is no evaluation, and a pass that denied every proposal is kept apart.
#[test]
fn skipped_and_denied_planner_passes_are_not_evaluations_that_found_no_action_indicated() {
    use crate::autotune::planner::{PlannerNoActionSummary, PlannerSummary};
    let no_action = |reason: &str| PlannerSummary {
        no_action: Some(PlannerNoActionSummary {
            reason: reason.into(),
            total_proposals: 0,
            eligible_proposals: 0,
            grouped_denials: Vec::new(),
            top_denied_candidates: Vec::new(),
            missing_capabilities: Vec::new(),
            workload_blocked: Vec::new(),
            manual_only_suggestions: Vec::new(),
        }),
        ..Default::default()
    };
    let mut observer = ResidentObserver::default();
    observer.arm(target(), Some(0), 1_000);
    // Skipped passes: nothing is counted.
    observer.observe_decision(
        Some(4242),
        Some(&no_action("observe mode does not suggest or apply")),
    );
    observer.observe_decision(Some(4242), Some(&no_action("focus is idle or unknown")));
    // The planner ran and proposed nothing for the process: an evaluation that found no action indicated.
    observer.observe_decision(Some(4242), Some(&no_action("no candidate was eligible")));
    // The planner ran and a capability the action needs was missing: a refusal, kept apart.
    observer.observe_decision(
        Some(4242),
        Some(&PlannerSummary {
            missing_capabilities: vec!["cpu_affinity".into()],
            ..no_action("no candidate was eligible")
        }),
    );
    let window = observer.freeze(1_100).remove(0);
    assert_eq!((window.evaluations, window.denied_evaluations), (1, 1));
}

/// A scheduler interval does not say who owns the load: an interval record with no process identity leaves the window
/// with unattributed owners, which the observation producer refuses to report as identified.
#[test]
fn an_interval_with_no_process_identity_is_counted_as_an_unattributed_owner() {
    let mut observer = ResidentObserver::default();
    observer.arm(target(), Some(0), 1_000);
    let mut anonymous = interval(4242, TaskClass::Compiler, 50, 1_000_000);
    anonymous.process_pid = None;
    observer.observe_intervals(
        &[interval(4242, TaskClass::Game, 50, 500_000), anonymous],
        0,
    );
    let window = observer.freeze(1_100).remove(0);
    assert_eq!(window.unattributed_intervals, 1);
    assert!(window.cpu_samples > 0);
}

/// One epoch per interval event of the target, stamped with the time the runtime observed it, in the canonical layout:
/// what lets the stretch while an action was held be cut out of the attempt exactly. A window that outgrows the bound
/// says so instead of silently dropping epochs.
#[test]
fn each_interval_event_of_the_target_is_kept_as_an_epoch_and_a_window_that_outgrows_the_bound_says_so()
 {
    use crate::autotune::research::action_epochs::MAX_EPOCHS;
    let mut observer = ResidentObserver::default();
    observer.arm(target(), Some(0), 1_000);
    for at in [1_010, 1_020, 1_030] {
        observer.observe_intervals_at(
            &[
                interval(4242, TaskClass::Game, 100, 500_000),
                // Another class is not the game's population.
                interval(9, TaskClass::Compiler, 5_000, 90_000_000),
            ],
            0,
            at,
        );
    }
    // An event with nothing of the target in it is no epoch.
    observer.observe_intervals_at(&[interval(9, TaskClass::Compiler, 5, 1)], 0, 1_040);
    let window = observer.freeze(1_100).remove(0);
    assert!(!window.epochs_truncated);
    assert_eq!(
        window
            .epochs
            .iter()
            .map(|e| (e.unix, e.events))
            .collect::<Vec<_>>(),
        [(1_010, 100), (1_020, 100), (1_030, 100)]
    );
    // The epochs add up to the window's own population.
    let total: u64 = window.epochs.iter().map(|e| e.events).sum();
    assert_eq!(total, window.events());
    // Past the bound the window is marked, and keeps counting its population.
    let mut long = ResidentObserver::default();
    long.arm(target(), Some(0), 1_000);
    for at in 0..=MAX_EPOCHS as u64 {
        long.observe_intervals_at(
            &[interval(4242, TaskClass::Game, 1, 500_000)],
            0,
            1_000 + at,
        );
    }
    let window = long.freeze(1_000_000).remove(0);
    assert!(window.epochs_truncated);
    assert_eq!(window.epochs.len(), MAX_EPOCHS);
}

/// The host facts are looked at once per interval in which the game was seen, from the files under the observer's root.
#[test]
fn each_interval_the_game_was_seen_in_looks_at_the_hosts_own_files() {
    let host = tempfile::tempdir().unwrap();
    let put = |relative: &str, text: &str| {
        let path = host.path().join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    put("sys/devices/system/cpu/online", "0-11\n");
    put(
        "sys/devices/system/cpu/cpu0/topology/thread_siblings_list",
        "0,6\n",
    );
    put("sys/devices/system/node/online", "0\n");
    put("proc/interrupts", "CPU0\n 0: 1 timer\n");
    put("proc/irq/default_smp_affinity", "fff\n");
    let mut observer = ResidentObserver::default().with_host_root(host.path().to_path_buf());
    observer.arm(target(), Some(0), 1_010);
    for _ in 0..12 {
        observer.observe_intervals(&[interval(4242, TaskClass::Game, 100, 500_000)], 0);
    }
    let window = observer.freeze(1_090).remove(0);
    // Topology and the single node were readable every time; the memory files were never there.
    assert_eq!(
        (
            window.host_samples["topology_samples"],
            window.host_samples["single_numa_samples"],
            window.host_samples["memory_guard_samples"],
            window.host_samples["irq_placement_samples"],
            window.host_samples["power_policy_samples"],
        ),
        (12, 12, 0, 12, 0)
    );
}

/// A tool's owner state counts only when its bus answered (running or not running), never when it could not be read.
#[test]
fn an_owner_state_counts_only_when_the_bus_gave_a_definite_answer() {
    use crate::integrations::exact::{Exact, ExactOwnerStates};
    let mut observer = ResidentObserver::default();
    observer.arm(target(), Some(0), 1_010);
    let states = ExactOwnerStates {
        gamemode_clients: Exact::Absent,
        scx_loader_scheduler: Exact::Present("lavd".into()),
        power_profile: Exact::Unreachable("system bus: NoBusAddress".into()),
    };
    for _ in 0..3 {
        observer.observe_owner_states(&states);
    }
    let window = observer.freeze(1_090).remove(0);
    let count = |name: &str| window.host_samples.get(name).copied();
    assert_eq!(count("owner_gamemode_samples"), Some(3));
    assert_eq!(count("owner_scx_loader_samples"), Some(3));
    assert_eq!(count("owner_power_profile_samples"), None);
}
