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
        Profile::Full => Timing {
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
    fn a_worker_kill_always_runs_where_the_worker_matters() {
        for round in CYCLE.iter().filter(|round| round.kind == WorkerKill) {
            assert_eq!(round.operations, Cgroup);
        }
    }
}
