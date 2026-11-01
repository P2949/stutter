//! The deterministic round schedule. Round `n` of a soak is a pure function of the profile, so a
//! resumed soak continues the same sequence, and the minimum counts of the acceptance list are
//! guaranteed by construction rather than hoped for.
use super::records::Profile;

/// What the workload does to the target's CPU, and so which way an experiment should go.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scenario {
    /// Contention on the game's CPU only: moving the game helps, an experiment is kept.
    Beneficial,
    /// The same contention on both CPUs: moving cannot help, an experiment is reverted.
    Neutral,
    /// Heavier contention on the destination: moving hurts, an experiment is reverted.
    Regression,
}

impl Scenario {
    pub fn name(self) -> &'static str {
        match self {
            Self::Beneficial => "beneficial",
            Self::Neutral => "neutral",
            Self::Regression => "regression",
        }
    }

    /// SCHED_FIFO burst milliseconds per 20 on (game CPU, destination CPU).
    pub fn bursts(self) -> (u64, u64) {
        match self {
            Self::Beneficial => (8, 0),
            Self::Neutral => (8, 8),
            Self::Regression => (2, 10),
        }
    }
}

/// Which graduated operations the controller may choose in a round.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operations {
    Affinity,
    Cgroup,
    Both,
}

impl Operations {
    pub fn name(self) -> &'static str {
        match self {
            Self::Affinity => "cpu_affinity_profile",
            Self::Cgroup => "cgroup_placement",
            Self::Both => "both",
        }
    }
}

/// The event a round exercises beyond a plain experiment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    /// One experiment, then the workload exits normally.
    Plain,
    /// `kill -9` the controller mid-experiment and start another on the same workload.
    ControllerCrash,
    /// SIGINT the controller mid-experiment, then start another on the same workload.
    ControllerStop,
    /// `kill -9` the privileged worker mid-experiment, then restart it.
    WorkerKill,
    /// Suspend the machine mid-experiment and resume it.
    Suspend,
    /// `kill -9` the workload mid-experiment.
    WorkloadKill,
    /// One workload exits and another becomes the focus.
    FocusSwap,
    /// A long-lived workload: several experiments, cooldowns, conflicts and candidate memory.
    Long,
    /// Service only: `systemctl restart` while a candidate is applied.
    ServiceRestart,
    /// Service only: SIGTERM of the controller while a candidate is applied.
    ControllerSigterm,
    /// Service only: the only workload is an unqualified game.
    FocusUnqualified,
    /// Service only: another owner changes the task Stutter changed.
    ExternalMutation,
    /// Service only: the controller journal is corrupted after a crash.
    CorruptJournal,
    /// Service only: the cgroup the task came from no longer exists to restore into.
    FailedRestore,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::ControllerCrash => "controller_crash",
            Self::ControllerStop => "controller_stop",
            Self::WorkerKill => "worker_kill",
            Self::Suspend => "suspend",
            Self::WorkloadKill => "workload_kill",
            Self::FocusSwap => "focus_swap",
            Self::Long => "long",
            Self::ServiceRestart => "service_restart",
            Self::ControllerSigterm => "controller_sigterm",
            Self::FocusUnqualified => "focus_unqualified",
            Self::ExternalMutation => "external_mutation",
            Self::CorruptJournal => "corrupt_journal",
            Self::FailedRestore => "failed_restore",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Round {
    pub kind: Kind,
    pub scenario: Scenario,
    pub operations: Operations,
}

use Kind::{
    ControllerCrash, ControllerStop, FocusSwap, Long, Plain, Suspend, WorkerKill, WorkloadKill,
};
use Operations::{Affinity, Both, Cgroup};
use Scenario::{Beneficial, Neutral, Regression};

const fn round(kind: Kind, scenario: Scenario, operations: Operations) -> Round {
    Round {
        kind,
        scenario,
        operations,
    }
}

/// One cycle. Every graduated operation is kept and reverted, every fault is injected on each
/// operation that can see it, and the long round exercises cooldown and conflicts.
const CYCLE: [Round; 16] = [
    round(Plain, Beneficial, Affinity),
    round(Plain, Beneficial, Cgroup),
    round(Plain, Regression, Affinity),
    round(Plain, Neutral, Cgroup),
    round(ControllerCrash, Beneficial, Affinity),
    round(WorkerKill, Beneficial, Cgroup),
    round(Plain, Beneficial, Both),
    round(ControllerStop, Beneficial, Cgroup),
    round(Suspend, Beneficial, Affinity),
    round(FocusSwap, Beneficial, Both),
    round(WorkloadKill, Beneficial, Affinity),
    round(ControllerCrash, Beneficial, Cgroup),
    round(Suspend, Beneficial, Cgroup),
    round(Long, Beneficial, Both),
    round(WorkerKill, Regression, Cgroup),
    round(Plain, Neutral, Affinity),
];

/// Rounds in one schedule cycle: the unit of the full profile's clean-cycle coverage.
pub const CYCLE_LEN: usize = CYCLE.len();

/// The service-only perturbations. They follow every historical cycle in the service profile and
/// never change the 16-round schedule the direct-controller soak was qualified with.
const SERVICE_OVERLAY: [Kind; 6] = [
    Kind::ServiceRestart,
    Kind::ControllerSigterm,
    Kind::FocusUnqualified,
    Kind::ExternalMutation,
    Kind::CorruptJournal,
    Kind::FailedRestore,
];

/// Rounds in one service cycle: the historical cycle then the service overlay.
pub const SERVICE_CYCLE_LEN: usize = CYCLE.len() + SERVICE_OVERLAY.len();

/// The event kinds the service profile must repeat, one per overlay round.
pub const SERVICE_EVENT_KINDS: [&str; 6] = [
    "service_restart",
    "controller_sigterm",
    "focus_unqualified",
    "external_mutation",
    "corrupt_journal",
    "failed_restore",
];

/// Round `index` of the service profile. Overlay operations alternate by cycle so both graduated
/// operations meet an external owner and a SIGTERM.
pub fn service_round_for(index: u32) -> Round {
    let slot = (index as usize) % SERVICE_CYCLE_LEN;
    if slot < CYCLE.len() {
        return CYCLE[slot];
    }
    let cycle = (index as usize) / SERVICE_CYCLE_LEN;
    let kind = SERVICE_OVERLAY[slot - CYCLE.len()];
    let alternating = if cycle % 2 == 0 { Affinity } else { Cgroup };
    let operations = match kind {
        Kind::ExternalMutation | Kind::ControllerSigterm => alternating,
        Kind::FailedRestore => Cgroup,
        _ => Both,
    };
    round(kind, Beneficial, operations)
}

/// The schedule kinds that inject a fault or run long; each has its own coverage floor. A
/// `plain` round injects nothing and is not listed.
pub const DANGEROUS_KINDS: [&str; 7] = [
    "controller_crash",
    "controller_stop",
    "worker_kill",
    "suspend",
    "workload_kill",
    "focus_swap",
    "long",
];

pub fn round_for(index: u32) -> Round {
    CYCLE[(index as usize) % CYCLE.len()]
}

/// How long the controller's candidate window is, and how long a long round keeps its workload.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    /// `--duration-seconds` of an apply session: the candidate window.
    pub candidate_window_s: u64,
    /// The longest a round waits for one experiment to resolve.
    pub resolve_timeout_s: u64,
    pub long_round_s: u64,
    pub suspend_s: u64,
}

pub fn timing(profile: Profile) -> Timing {
    match profile {
        Profile::Full | Profile::Service => Timing {
            candidate_window_s: 30,
            resolve_timeout_s: 240,
            long_round_s: 540,
            suspend_s: 30,
        },
        Profile::Smoke => Timing {
            candidate_window_s: 12,
            resolve_timeout_s: 150,
            long_round_s: 330,
            suspend_s: 20,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_schedule_is_deterministic_and_cyclic() {
        for index in 0..64 {
            assert_eq!(round_for(index), round_for(index + CYCLE.len() as u32));
        }
    }

    #[test]
    fn one_cycle_covers_every_event_and_both_operations() {
        let kinds: Vec<Kind> = CYCLE.iter().map(|round| round.kind).collect();
        for kind in [
            Plain,
            ControllerCrash,
            ControllerStop,
            WorkerKill,
            Suspend,
            WorkloadKill,
            FocusSwap,
            Long,
        ] {
            assert!(kinds.contains(&kind), "{kind:?}");
        }
        let count = |predicate: fn(&Round) -> bool| CYCLE.iter().filter(|r| predicate(r)).count();
        assert!(count(|r| r.kind == ControllerCrash) >= 2);
        assert!(count(|r| r.kind == WorkerKill) >= 2);
        assert!(count(|r| r.kind == Suspend) >= 2);
        assert!(count(|r| r.scenario == Beneficial) >= 8);
        assert!(count(|r| r.scenario != Beneficial) >= 3);
        assert!(count(|r| r.operations == Cgroup) >= 5);
        assert!(count(|r| r.operations == Affinity) >= 5);
        assert!(count(|r| r.operations == Both) >= 3);
    }

    #[test]
    fn the_dangerous_kinds_are_exactly_the_non_plain_kinds_of_the_schedule() {
        let mut scheduled: Vec<&str> = CYCLE
            .iter()
            .map(|round| round.kind.name())
            .filter(|name| *name != "plain")
            .collect();
        scheduled.sort_unstable();
        scheduled.dedup();
        let mut listed = DANGEROUS_KINDS.to_vec();
        listed.sort_unstable();
        assert_eq!(scheduled, listed);
        assert_eq!(CYCLE_LEN, 16);
        // Six cycles give every dangerous kind its floor of six.
        for kind in DANGEROUS_KINDS {
            assert!(
                CYCLE.iter().any(|round| round.kind.name() == kind),
                "{kind}"
            );
        }
    }

    #[test]
    fn a_worker_kill_always_runs_where_the_worker_matters() {
        for round in CYCLE.iter().filter(|round| round.kind == WorkerKill) {
            assert_eq!(round.operations, Cgroup);
        }
    }
}

#[cfg(test)]
mod service_tests {
    use super::*;

    #[test]
    fn the_historical_cycle_is_untouched_inside_the_service_cycle() {
        for index in 0..CYCLE.len() as u32 {
            assert_eq!(service_round_for(index), round_for(index));
        }
    }

    #[test]
    fn six_service_cycles_repeat_every_service_event_at_least_six_times() {
        let mut counts = std::collections::BTreeMap::new();
        for index in 0..(6 * SERVICE_CYCLE_LEN) as u32 {
            *counts
                .entry(service_round_for(index).kind.name())
                .or_insert(0_usize) += 1;
        }
        for event in SERVICE_EVENT_KINDS {
            assert!(counts.get(event).copied().unwrap_or(0) >= 6, "{event}");
        }
    }

    #[test]
    fn external_owners_and_sigterm_meet_both_graduated_operations() {
        for kind in [Kind::ExternalMutation, Kind::ControllerSigterm] {
            let operations: std::collections::BTreeSet<_> = (0..(2 * SERVICE_CYCLE_LEN) as u32)
                .map(service_round_for)
                .filter(|round| round.kind == kind)
                .map(|round| round.operations.name())
                .collect();
            assert_eq!(operations.len(), 2, "{kind:?}");
        }
    }

    #[test]
    fn every_service_event_name_is_a_service_round_kind() {
        let names: Vec<_> = SERVICE_OVERLAY.iter().map(|kind| kind.name()).collect();
        assert_eq!(names, SERVICE_EVENT_KINDS.to_vec());
    }
}
