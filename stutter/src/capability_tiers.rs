//! User-facing capability boundary. A capability is `Supported` only when the live
//! evidence it names exists and verifies; `xtask capability-check` enforces that
//! against `docs/evidence/index.json`, so the tier cannot drift ahead of the evidence.
use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityTier {
    /// Contract, regression coverage and real-system evidence exist; documented limits.
    Supported,
    /// Usable but not yet backed by the evidence its target tier requires.
    Experimental,
    /// Implementation detail or test support; not a user surface.
    Internal,
    /// Present in the tree but refused at runtime.
    Disabled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityClass {
    NonMutating,
    Mutating,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Capability {
    pub id: &'static str,
    pub title: &'static str,
    pub class: CapabilityClass,
    /// Tier justified by the evidence that exists today.
    pub tier: CapabilityTier,
    /// Tier intended for the first supported release.
    pub target_tier: CapabilityTier,
    pub commands: &'static [&'static str],
    /// Mutation provider kinds this capability governs (see `MUTATION_PROVIDER_KINDS`).
    pub providers: &'static [&'static str],
    /// Live-acceptance suites that must have a verified PASS bundle to be `Supported`.
    pub required_evidence: &'static [&'static str],
    pub limitations: &'static str,
}

/// Required for every mutating capability before it can be `Supported`.
pub const MUTATING_REQUIRED_EVIDENCE: [&str; 2] = ["live_recovery", "live_suspend_resume"];

const OBSERVATION: &[&str] = &["live_observation"];
const AFFINITY: &[&str] = &[
    "live_observation",
    "live_affinity",
    "live_recovery",
    "live_suspend_resume",
];

pub const CAPABILITIES: [Capability; 17] = [
    obs(
        "doctor",
        "Local preflight checks",
        &["doctor"],
        CapabilityTier::Experimental,
        CapabilityTier::Supported,
    ),
    obs(
        "record",
        "Record scheduler and frame evidence",
        &["record"],
        CapabilityTier::Experimental,
        CapabilityTier::Supported,
    ),
    obs(
        "bench",
        "Repeatable benchmark scenarios",
        &["bench", "scenario"],
        CapabilityTier::Experimental,
        CapabilityTier::Supported,
    ),
    obs(
        "report",
        "Report and HTML rendering of recordings",
        &["report", "summary", "validate"],
        CapabilityTier::Experimental,
        CapabilityTier::Supported,
    ),
    obs(
        "diagnose",
        "Evidence-based stutter diagnosis",
        &["report", "check"],
        CapabilityTier::Experimental,
        CapabilityTier::Supported,
    ),
    obs(
        "advisor",
        "Advisor and fix-plan recommendations",
        &["advisor", "recommend"],
        CapabilityTier::Experimental,
        CapabilityTier::Supported,
    ),
    obs(
        "tune_evaluation",
        "Baseline/candidate evaluation of tunings",
        &["tune", "recommend", "prove-fix"],
        CapabilityTier::Experimental,
        CapabilityTier::Supported,
    ),
    obs(
        "audit",
        "Inspect local audit events",
        &["audit"],
        CapabilityTier::Experimental,
        CapabilityTier::Supported,
    ),
    Capability {
        id: "restore",
        title: "Restore recorded CPU-affinity state",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Experimental,
        target_tier: CapabilityTier::Supported,
        commands: &["restore"],
        providers: &[],
        required_evidence: &["live_affinity", "live_recovery", "live_suspend_resume"],
        limitations: "Restores only the exact process/task identities recorded; replacement identities are skipped.",
    },
    Capability {
        id: "cpu_affinity_experiment",
        title: "Explicit reversible CPU-affinity experiment",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Experimental,
        target_tier: CapabilityTier::Supported,
        commands: &["tune", "autotune"],
        providers: &["cpu_affinity_profile"],
        required_evidence: AFFINITY,
        limitations: "Low-risk, per-process-tree scope only; benefit is not claimed without calibration evidence.",
    },
    Capability {
        id: "process_priority_and_placement",
        title: "nice, ioprio, uclamp and cgroup placement",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Experimental,
        target_tier: CapabilityTier::Experimental,
        commands: &["autotune"],
        providers: &["nice", "ionice", "uclamp", "cgroup_placement"],
        required_evidence: &[],
        limitations: "Medium-risk; does not inherit CPU-affinity maturity.",
    },
    Capability {
        id: "system_wide_tuning",
        title: "IRQ affinity, CPU/GPU power and VM settings",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Experimental,
        target_tier: CapabilityTier::Experimental,
        commands: &["autotune"],
        providers: &["irq_affinity", "cpu_power", "gpu_power", "vm_knob"],
        required_evidence: &[],
        limitations: "System-wide effects; manual/privileged only; not part of the first supported release.",
    },
    Capability {
        id: "always_on_autotune",
        title: "Always-on autotune daemon and service",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Experimental,
        target_tier: CapabilityTier::Experimental,
        commands: &["daemon", "service", "autotune-status"],
        providers: &[],
        required_evidence: &[],
        limitations: "Long-running mutation; gated by daemon policy; not supported in the first release.",
    },
    Capability {
        id: "remote_agent_mutation",
        title: "Remote agent endpoints",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Experimental,
        target_tier: CapabilityTier::Experimental,
        commands: &["agent"],
        providers: &[],
        required_evidence: &[],
        limitations: "Remote mutation remains experimental.",
    },
    Capability {
        id: "privileged_worker",
        title: "Privileged worker used by service paths",
        class: CapabilityClass::Mutating,
        tier: CapabilityTier::Internal,
        target_tier: CapabilityTier::Internal,
        commands: &["privileged-worker"],
        providers: &[],
        required_evidence: &[],
        limitations: "Implementation detail of the service path; not a user surface.",
    },
    Capability {
        id: "fake_action_provider",
        title: "Synthetic test action provider",
        class: CapabilityClass::NonMutating,
        tier: CapabilityTier::Internal,
        target_tier: CapabilityTier::Internal,
        commands: &[],
        providers: &[],
        required_evidence: &[],
        limitations: "Test support only.",
    },
    obs(
        "completions_and_man",
        "Shell completions and manual page",
        &["completions", "man"],
        CapabilityTier::Experimental,
        CapabilityTier::Supported,
    ),
];

const fn obs(
    id: &'static str,
    title: &'static str,
    commands: &'static [&'static str],
    tier: CapabilityTier,
    target_tier: CapabilityTier,
) -> Capability {
    Capability {
        id,
        title,
        class: CapabilityClass::NonMutating,
        tier,
        target_tier,
        commands,
        providers: &[],
        required_evidence: OBSERVATION,
        limitations: match tier {
            CapabilityTier::Supported => {
                "Observation is useful without mutation; backed by indexed real-system observation, kernel and overhead evidence."
            }
            _ => {
                "Observation is useful without mutation; real-system evidence is pending (see docs/evidence/index.json)."
            }
        },
    }
}

pub fn capability_registry() -> &'static [Capability] {
    &CAPABILITIES
}

/// Human-readable listing for `stutter capabilities`.
pub fn render_capabilities_text() -> String {
    let mut text =
        String::from("Capability tiers (Supported / Experimental / Internal / Disabled)\n");
    for capability in capability_registry() {
        text.push_str(&format!(
            "- {} [{:?}, target {:?}] {}\n    class: {:?}; commands: {}\n    required live evidence: {}\n    {}\n",
            capability.id,
            capability.tier,
            capability.target_tier,
            capability.title,
            capability.class,
            if capability.commands.is_empty() { "-".to_owned() } else { capability.commands.join(", ") },
            if capability.required_evidence.is_empty() { "-".to_owned() } else { capability.required_evidence.join(", ") },
            capability.limitations,
        ));
    }
    text.push_str("A capability is Supported only when its required live evidence is verified.\n");
    text
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::autotune::planning::candidate::MUTATION_PROVIDER_KINDS;

    #[test]
    fn ids_are_unique() {
        let ids = capability_registry()
            .iter()
            .map(|c| c.id)
            .collect::<BTreeSet<_>>();
        assert_eq!(ids.len(), capability_registry().len());
    }

    #[test]
    fn every_mutation_provider_is_governed_by_exactly_one_capability() {
        for kind in MUTATION_PROVIDER_KINDS {
            let owners = capability_registry()
                .iter()
                .filter(|capability| capability.providers.contains(&kind))
                .count();
            assert_eq!(owners, 1, "provider {kind} must have exactly one owner");
        }
        for capability in capability_registry() {
            for provider in capability.providers {
                assert!(
                    MUTATION_PROVIDER_KINDS.contains(provider),
                    "unknown provider {provider}"
                );
            }
            if capability.class == CapabilityClass::NonMutating {
                assert!(capability.providers.is_empty(), "{}", capability.id);
            }
        }
    }

    #[test]
    fn providers_do_not_inherit_cpu_affinity_maturity() {
        for capability in capability_registry()
            .iter()
            .filter(|c| c.id != "cpu_affinity_experiment")
        {
            assert!(!capability.providers.contains(&"cpu_affinity_profile"));
            if !capability.providers.is_empty() {
                assert_ne!(
                    capability.target_tier,
                    CapabilityTier::Supported,
                    "{}",
                    capability.id
                );
            }
        }
    }

    #[test]
    fn supported_requires_evidence_limitations_and_mutating_recovery_suites() {
        for capability in capability_registry() {
            assert!(
                !capability.limitations.trim().is_empty(),
                "{}",
                capability.id
            );
            if capability.tier == CapabilityTier::Supported {
                assert_eq!(capability.target_tier, CapabilityTier::Supported);
            }
            if capability.target_tier == CapabilityTier::Supported {
                assert!(
                    !capability.required_evidence.is_empty(),
                    "{}",
                    capability.id
                );
                if capability.class == CapabilityClass::Mutating {
                    for suite in MUTATING_REQUIRED_EVIDENCE {
                        assert!(
                            capability.required_evidence.contains(&suite),
                            "{} must require {suite}",
                            capability.id
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn no_mutating_capability_is_supported_in_the_first_release() {
        // Which non-mutating capabilities may be Supported is decided by evidence
        // (`xtask capability-check`); mutation never graduates implicitly.
        assert!(capability_registry().iter().all(|capability| {
            capability.tier != CapabilityTier::Supported
                || capability.class == CapabilityClass::NonMutating
        }));
    }

    #[test]
    fn text_listing_names_every_capability_and_the_rule() {
        let text = render_capabilities_text();
        for capability in capability_registry() {
            assert!(text.contains(capability.id));
        }
        assert!(text.contains("Supported only when"));
    }
}
