//! Named fault scenarios plus a seeded random walk. Every scenario runs the same invariant check
//! after every event, so a fault cannot hide a lost token behind a passing assertion.
use std::time::Duration;

use super::{Event, Proposal, Response, Sim, host::Fault};
use crate::autotune::conflicts::ActionConflictGroup;

fn play(sim: &mut Sim, events: &[Event]) {
    for event in events {
        if let Err(violation) = sim.run(*event) {
            panic!("{violation} after {event:?}\n{}", sim.report());
        }
    }
}

/// Evaluate until the active experiment resolves (keep or revert), within a bounded horizon.
fn resolve(sim: &mut Sim) {
    for _ in 0..40 {
        if !sim.manager.has_active_experiment() {
            return;
        }
        play(sim, &[Event::Tick(None)]);
    }
    panic!("experiment did not resolve: {}", sim.report());
}

/// Let any cooldown elapse without proposing anything.
fn cool_down(sim: &mut Sim) {
    for _ in 0..40 {
        play(sim, &[Event::Tick(None)]);
    }
}

fn last(sim: &Sim) -> &str {
    &sim.trail.last().expect("an event ran").what
}

fn kept_groups(sim: &Sim) -> Vec<ActionConflictGroup> {
    sim.profile
        .kept_actions()
        .map(|(group, _)| *group)
        .collect()
}

fn live_effects(sim: &Sim) -> usize {
    sim.host.borrow().live_effects().count()
}

#[test]
fn an_improving_experiment_is_kept_and_restored_on_shutdown() {
    let mut sim = Sim::new("keep-shutdown");
    play(&mut sim, &[Event::Tick(Some(Proposal::Affinity))]);
    assert_eq!(last(&sim), "started");
    resolve(&mut sim);
    assert_eq!(kept_groups(&sim), vec![ActionConflictGroup::CpuPlacement]);
    assert_eq!(live_effects(&sim), 1);
    play(&mut sim, &[Event::Shutdown]);
    assert_eq!(live_effects(&sim), 0, "{}", sim.report());
}

#[test]
fn a_regressing_experiment_is_reverted_and_cooled_down() {
    let mut sim = Sim::new("revert-cooldown");
    play(
        &mut sim,
        &[
            Event::Respond(Response::Regresses),
            Event::Tick(Some(Proposal::Affinity)),
        ],
    );
    resolve(&mut sim);
    assert!(kept_groups(&sim).is_empty());
    assert_eq!(live_effects(&sim), 0);
    // The same action is offered again inside the cooldown: the controller must not start it.
    play(&mut sim, &[Event::Tick(Some(Proposal::Affinity))]);
    assert!(!sim.manager.has_active_experiment(), "{}", sim.report());
}

#[test]
fn conflicting_groups_never_coexist_but_compatible_ones_do() {
    let mut sim = Sim::new("conflicts");
    play(&mut sim, &[Event::Tick(Some(Proposal::Affinity))]);
    resolve(&mut sim);
    cool_down(&mut sim);
    // cgroup placement conflicts with the kept CPU placement: the planner never offers it.
    play(&mut sim, &[Event::Tick(Some(Proposal::Cgroup))]);
    assert!(!sim.manager.has_active_experiment());
    // A compatible family can be kept beside it, one experiment at a time.
    play(&mut sim, &[Event::Tick(Some(Proposal::Coexisting(1)))]);
    resolve(&mut sim);
    assert_eq!(kept_groups(&sim).len(), 2, "{}", sim.report());
    play(&mut sim, &[Event::Shutdown]);
    assert_eq!(live_effects(&sim), 0, "{}", sim.report());
}

#[test]
fn an_unsupported_operation_is_refused_before_any_write() {
    let mut sim = Sim::new("unsupported");
    play(&mut sim, &[Event::Tick(Some(Proposal::Nice))]);
    assert!(last(&sim).starts_with("error"), "{}", last(&sim));
    assert_eq!(sim.host.borrow().mutations, 0);
    assert!(!sim.manager.has_active_experiment());
}

#[test]
fn the_workload_exiting_reverts_the_active_experiment() {
    let mut sim = Sim::new("workload-exit");
    play(&mut sim, &[Event::Tick(Some(Proposal::Affinity))]);
    play(&mut sim, &[Event::WorkloadStop]);
    resolve(&mut sim);
    assert!(!sim.manager.has_active_experiment(), "{}", sim.report());
    assert_eq!(live_effects(&sim), 0);
}

#[test]
fn a_focus_change_reverts_the_active_experiment() {
    let mut sim = Sim::new("focus");
    play(&mut sim, &[Event::Tick(Some(Proposal::Affinity))]);
    play(&mut sim, &[Event::Focus(false)]);
    resolve(&mut sim);
    assert!(!sim.manager.has_active_experiment(), "{}", sim.report());
    assert_eq!(live_effects(&sim), 0);
}

#[test]
fn a_suspend_gap_reverts_the_active_experiment() {
    let mut sim = Sim::new("suspend");
    play(&mut sim, &[Event::Tick(Some(Proposal::Cgroup))]);
    play(&mut sim, &[Event::Suspend(Duration::from_secs(3_600))]);
    assert!(!sim.manager.has_active_experiment());
    assert_eq!(live_effects(&sim), 0);
}

#[test]
fn an_external_writer_is_never_overwritten_by_a_restore() {
    let mut sim = Sim::new("external");
    play(&mut sim, &[Event::Tick(Some(Proposal::Affinity))]);
    play(
        &mut sim,
        &[Event::External(ActionConflictGroup::CpuPlacement)],
    );
    let external = sim.host.borrow().value("CpuPlacement");
    play(&mut sim, &[Event::Shutdown]);
    assert_eq!(sim.host.borrow().value("CpuPlacement"), external);
}

#[test]
fn a_refused_apply_leaves_the_host_untouched() {
    let mut sim = Sim::new("apply-refused");
    play(
        &mut sim,
        &[
            Event::Fault(Fault::ApplyRefused),
            Event::Tick(Some(Proposal::Affinity)),
        ],
    );
    assert_eq!(sim.host.borrow().mutations, 0);
    assert_eq!(live_effects(&sim), 0);
}

#[test]
fn a_worker_dying_after_the_write_leaves_a_recoverable_token() {
    for proposal in [Proposal::Affinity, Proposal::Cgroup] {
        let mut sim = Sim::new("worker-death");
        play(
            &mut sim,
            &[
                Event::Fault(Fault::WorkerDiesAfterWrite),
                Event::Tick(Some(proposal)),
            ],
        );
        // The write landed and the worker is gone: the token must still be durable.
        assert_eq!(live_effects(&sim), 1, "{proposal:?}: {}", sim.report());
        play(&mut sim, &[Event::WorkerRestart, Event::DaemonRestart]);
        assert_eq!(live_effects(&sim), 0, "{proposal:?}: {}", sim.report());
    }
}

#[test]
fn daemon_death_in_every_phase_is_recovered_on_restart() {
    for ticks in 0..7 {
        let mut sim = Sim::new("daemon-death");
        play(&mut sim, &[Event::Tick(Some(Proposal::Cgroup))]);
        for _ in 0..ticks {
            play(&mut sim, &[Event::Tick(None)]);
        }
        play(&mut sim, &[Event::DaemonRestart]);
        assert_eq!(
            live_effects(&sim),
            0,
            "after {ticks} ticks: {}",
            sim.report()
        );
    }
}

#[test]
fn daemon_death_with_several_kept_actions_restores_all_of_them() {
    let mut sim = Sim::new("daemon-death-kept");
    play(&mut sim, &[Event::Tick(Some(Proposal::Affinity))]);
    resolve(&mut sim);
    cool_down(&mut sim);
    play(&mut sim, &[Event::Tick(Some(Proposal::Coexisting(1)))]);
    resolve(&mut sim);
    cool_down(&mut sim);
    play(&mut sim, &[Event::Tick(Some(Proposal::Coexisting(2)))]);
    play(&mut sim, &[Event::DaemonRestart]);
    assert_eq!(live_effects(&sim), 0, "{}", sim.report());
}

#[test]
fn a_failed_rollback_keeps_the_token_until_it_can_be_restored() {
    let mut sim = Sim::new("rollback-fails");
    play(
        &mut sim,
        &[
            Event::Respond(Response::Regresses),
            Event::Tick(Some(Proposal::Affinity)),
            Event::Fault(Fault::RollbackFails),
        ],
    );
    for _ in 0..40 {
        play(&mut sim, &[Event::Tick(None)]);
    }
    // Whatever the controller decided, nothing may be left live without a durable token.
    play(&mut sim, &[Event::DaemonRestart]);
    assert_eq!(live_effects(&sim), 0, "{}", sim.report());
}

#[test]
fn a_failed_exit_rollback_of_a_kept_action_is_not_forgotten() {
    let mut sim = Sim::new("exit-rollback-fails");
    play(&mut sim, &[Event::Tick(Some(Proposal::Affinity))]);
    resolve(&mut sim);
    play(
        &mut sim,
        &[Event::Fault(Fault::RollbackFails), Event::Shutdown],
    );
    assert_eq!(live_effects(&sim), 1);
    play(&mut sim, &[Event::DaemonRestart]);
    assert_eq!(live_effects(&sim), 0, "{}", sim.report());
}

#[test]
fn a_reboot_retires_tokens_instead_of_applying_them_to_the_new_boot() {
    let mut sim = Sim::new("reboot");
    play(&mut sim, &[Event::Tick(Some(Proposal::Cgroup))]);
    play(&mut sim, &[Event::Reboot]);
    assert_eq!(live_effects(&sim), 0);
    assert!(!sim.manager.has_active_experiment());
    assert_eq!(sim.host.borrow().value("CgroupPlacement"), 0);
}

#[test]
fn a_losing_candidate_is_not_retried_immediately() {
    let mut sim = Sim::new("candidate-memory");
    play(
        &mut sim,
        &[
            Event::Respond(Response::Regresses),
            Event::Tick(Some(Proposal::Affinity)),
        ],
    );
    resolve(&mut sim);
    let starts = sim.last_started.len();
    for _ in 0..4 {
        play(&mut sim, &[Event::Tick(Some(Proposal::Affinity))]);
        assert!(!sim.manager.has_active_experiment(), "{}", sim.report());
    }
    assert_eq!(sim.last_started.len(), starts);
}

/// A tiny xorshift: the random walk is reproducible from its seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self, bound: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % bound
    }
}

fn random_event(rng: &mut Rng) -> Event {
    match rng.next(24) {
        0..=7 => Event::Tick(Some(match rng.next(5) {
            0 => Proposal::Affinity,
            1 => Proposal::Cgroup,
            2 => Proposal::Nice,
            3 => Proposal::Coexisting(1),
            _ => Proposal::Coexisting(2),
        })),
        8..=11 => Event::Tick(None),
        12 => Event::Respond(match rng.next(3) {
            0 => Response::Improves,
            1 => Response::Neutral,
            _ => Response::Regresses,
        }),
        13 => Event::WorkloadStop,
        14 => Event::WorkloadStart,
        15 => Event::Focus(rng.next(2) == 0),
        16 => Event::External(ActionConflictGroup::CpuPlacement),
        17 => Event::Fault(match rng.next(4) {
            0 => Fault::ApplyRefused,
            1 => Fault::WorkerDiesAfterWrite,
            2 => Fault::RollbackFails,
            _ => Fault::WorkerDown,
        }),
        18 => Event::WorkerRestart,
        19 => Event::DaemonRestart,
        20 => Event::Shutdown,
        21 => Event::Suspend(Duration::from_secs(600)),
        22 => Event::Reboot,
        _ => Event::Tick(None),
    }
}

/// Seeded random walks: any seed that breaks an invariant is a reproducible bug report.
#[test]
fn random_fault_walks_keep_every_invariant() {
    // `STUTTER_FAULT_SIM_SEEDS=2000` widens the sweep when hunting for a counterexample.
    let seeds = std::env::var("STUTTER_FAULT_SIM_SEEDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(60_u64);
    for seed in 1..=seeds {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut sim = Sim::new("walk");
        for step in 0..70 {
            let event = random_event(&mut rng);
            if let Err(violation) = sim.run(event) {
                panic!(
                    "seed {seed} step {step}: {violation} after {event:?}\n{}",
                    sim.report()
                );
            }
        }
        // Every walk ends with a clean stop and a restart: nothing may stay applied.
        sim.host.borrow_mut().restart_worker();
        // The operator restarts a faulted controller until recovery succeeds; scripted faults are
        // one-shot, so it converges, and until then every token stays durable (checked each event).
        play(&mut sim, &[Event::Shutdown, Event::DaemonRestart]);
        for _ in 0..8 {
            if sim.up {
                break;
            }
            play(&mut sim, &[Event::DaemonRestart]);
        }
        assert!(
            sim.up,
            "seed {seed}: recovery never converged: {}",
            sim.report()
        );
        assert_eq!(live_effects(&sim), 0, "seed {seed}: {}", sim.report());
    }
}
