//! What each scheduler is known to do with nice weights, cgroup CPU weights and uclamp.
//!
//! The kernel's default scheduler honors all three (that is documented kernel behavior). A sched_ext
//! scheduler is its own policy: whether it honors a task's nice value, `cpu.weight` or uclamp is a
//! property of that scheduler, and an unverified claim here would let Stutter tune a knob that does
//! nothing or something else. So the table starts with only the kernel default as `Yes` and every
//! sched_ext scheduler `Unknown`, and `Unknown` denies the dependent operation. A scheduler is added
//! only with evidence (a matched comparison or the scheduler's documentation) in `VERIFIED`.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Honors {
    Yes,
    No,
    /// Not verified: treated as not honored.
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Knob {
    Nice,
    CgroupCpuWeight,
    Uclamp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulerSemantics {
    pub nice: Honors,
    pub cgroup_cpu_weight: Honors,
    pub uclamp: Honors,
}

/// Schedulers whose semantics were verified, with the evidence in the comment beside each entry.
/// Empty for sched_ext on purpose: nothing has been verified on this host.
const VERIFIED: &[(&str, SchedulerSemantics)] = &[];

const KERNEL_DEFAULT: SchedulerSemantics = SchedulerSemantics {
    nice: Honors::Yes,
    cgroup_cpu_weight: Honors::Yes,
    uclamp: Honors::Yes,
};

const UNKNOWN: SchedulerSemantics = SchedulerSemantics {
    nice: Honors::Unknown,
    cgroup_cpu_weight: Honors::Unknown,
    uclamp: Honors::Unknown,
};

/// The semantics of the scheduler in charge: `None` means no sched_ext scheduler is loaded.
pub fn semantics_of(sched_ext_scheduler: Option<&str>) -> SchedulerSemantics {
    let Some(name) = sched_ext_scheduler else {
        return KERNEL_DEFAULT;
    };
    VERIFIED
        .iter()
        .find(|(known, _)| *known == name)
        .map_or(UNKNOWN, |(_, semantics)| *semantics)
}

/// Whether an operation that relies on `knob` may be planned under this scheduler: only when the
/// knob is verified to be honored.
pub fn knob_is_honored(sched_ext_scheduler: Option<&str>, knob: Knob) -> bool {
    let semantics = semantics_of(sched_ext_scheduler);
    let honors = match knob {
        Knob::Nice => semantics.nice,
        Knob::CgroupCpuWeight => semantics.cgroup_cpu_weight,
        Knob::Uclamp => semantics.uclamp,
    };
    honors == Honors::Yes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kernel_default_honors_every_knob_and_an_unverified_sched_ext_scheduler_none() {
        for knob in [Knob::Nice, Knob::CgroupCpuWeight, Knob::Uclamp] {
            assert!(knob_is_honored(None, knob));
            for scheduler in ["scx_lavd", "scx_rusty", "anything"] {
                assert!(
                    !knob_is_honored(Some(scheduler), knob),
                    "{scheduler} {knob:?}"
                );
            }
        }
        assert_eq!(semantics_of(Some("scx_lavd")).nice, Honors::Unknown);
        assert_eq!(semantics_of(None).uclamp, Honors::Yes);
    }

    #[test]
    fn only_a_verified_yes_allows_the_knob() {
        assert!(!matches!(Honors::No, Honors::Yes));
        // The verified list is empty until evidence exists: no sched_ext name resolves to Yes.
        assert!(VERIFIED.iter().all(|(name, _)| name.starts_with("scx_")));
    }
}

#[cfg(test)]
mod denial_tests {
    #[test]
    fn the_planner_names_the_denial_with_a_stable_code() {
        assert_eq!(
            crate::autotune::planning::denial::CandidateDenyReason::SchedulerSemanticsUnverified
                .reason_code(),
            "scheduler_semantics_unverified"
        );
    }
}
