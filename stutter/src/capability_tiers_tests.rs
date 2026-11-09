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
fn every_mutation_provider_is_governed_and_every_operation_has_exactly_one_capability() {
    for kind in MUTATION_PROVIDER_KINDS {
        let owners = capability_registry()
            .iter()
            .filter(|capability| capability.providers.contains(&kind))
            .count();
        assert!(owners >= 1, "provider {kind} must be governed");
    }
    // Authority is per operation: each operation names one capability, and no two operations
    // share one, so one operation's maturity can never authorise another.
    let mut owners = BTreeSet::new();
    for operation in crate::autotune::operation::MutationOperationKind::ALL {
        let Some(id) = operation.capability_id() else {
            continue;
        };
        assert!(
            capability_registry().iter().any(|c| c.id == id),
            "{operation:?} names unknown capability {id}"
        );
        assert!(owners.insert(id), "{id} governs more than one operation");
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
        .filter(|c| !["cpu_affinity_experiment", "cgroup_placement_experiment"].contains(&c.id))
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
fn only_graduated_mutating_capabilities_are_supported() {
    // Which non-mutating capabilities may be Supported is decided by evidence
    // (`xtask capability-check`); mutation never graduates implicitly: each mutating
    // capability named here completed a graduation campaign.
    // The bounded controller joined with its indexed `live_autotune_soak` evidence; it
    // authorises nothing beyond the autonomous support registry.
    // The packaged bounded service joined on targeted acceptance; it adds no operation.
    const GRADUATED: &[&str] = &[
        "cpu_affinity_experiment",
        "cgroup_placement_experiment",
        "bounded_autotune_controller",
        "always_on_autotune",
    ];
    assert!(capability_registry().iter().all(|capability| {
        capability.tier != CapabilityTier::Supported
            || capability.class == CapabilityClass::NonMutating
            || GRADUATED.contains(&capability.id)
    }));
}

fn capability(id: &str) -> &'static Capability {
    capability_registry()
        .iter()
        .find(|capability| capability.id == id)
        .unwrap_or_else(|| panic!("no capability {id}"))
}

#[test]
fn the_bounded_controller_rests_on_the_soak_and_ships_on_the_smoke() {
    let controller = capability("bounded_autotune_controller");
    assert_eq!(controller.class, CapabilityClass::Mutating);
    assert_eq!(controller.target_tier, CapabilityTier::Supported);
    assert_eq!(controller.commands, ["autotune", "autotune-status"]);
    assert!(controller.required_evidence.contains(&"live_autotune_soak"));
    assert!(
        !controller
            .required_evidence
            .contains(&"live_autotune_smoke")
    );
    assert_eq!(
        controller.required_release_evidence,
        ["live_autotune_smoke"]
    );
    // It governs no operation: it authorises nothing the autonomous registry does not.
    assert!(controller.providers.is_empty());
    assert!(controller.limitations.contains("AUTONOMOUS_CONTRACTS"));
    // A release never re-demands the long qualification soak.
    assert!(capability_registry().iter().all(|capability| {
        !capability
            .required_release_evidence
            .contains(&"live_autotune_soak")
    }));
}

#[test]
fn graduated_mutation_capabilities_are_released_on_the_exact_rc_smoke() {
    for id in ["cpu_affinity_experiment", "cgroup_placement_experiment"] {
        assert_eq!(
            capability(id).required_release_evidence,
            ["live_observation", "live_autotune_smoke"],
            "{id}"
        );
        // Graduation proof is unchanged.
        assert!(capability(id).required_evidence.contains(&"live_recovery"));
    }
}

#[test]
fn the_bounded_service_rests_on_acceptance_and_claims_no_endurance() {
    let service = capability("always_on_autotune");
    assert_eq!(service.tier, CapabilityTier::Supported);
    assert!(
        service
            .required_evidence
            .contains(&"live_bounded_service_acceptance")
    );
    for suite in MUTATING_REQUIRED_EVIDENCE {
        assert!(service.required_evidence.contains(&suite), "{suite}");
    }
    // The endurance suite is a different contract and never stands in for acceptance.
    assert!(!service.required_evidence.contains(&"live_bounded_service"));
    // It governs no operation: it authorises nothing the autonomous registry does not.
    assert!(service.providers.is_empty());
    assert!(service.limitations.contains("AUTONOMOUS_CONTRACTS"));
    assert!(service.limitations.contains("extended endurance"));
}

#[test]
fn a_release_of_the_service_needs_its_own_exact_build_smoke() {
    let service = capability("always_on_autotune");
    assert!(
        service
            .required_release_evidence
            .contains(&"live_packaged_service_smoke")
    );
    // The short smoke is never graduation proof, and graduation never replaces it.
    assert!(
        !service
            .required_evidence
            .contains(&"live_packaged_service_smoke")
    );
}

#[test]
fn every_deferred_operation_and_the_remote_agent_stay_experimental() {
    for id in [
        "remote_agent_mutation",
        "uclamp_experiment",
        "nice_experiment",
        "ioprio_experiment",
        "cpu_power_epp",
        "cpu_power_governor",
        "gpu_power_profile",
        "gpu_force_performance",
        "vm_swappiness",
        "vm_dirty_background_ratio",
        "vm_dirty_ratio",
        "irq_affinity_known_device",
        "irq_affinity_storage_or_unknown",
    ] {
        assert_ne!(capability(id).tier, CapabilityTier::Supported, "{id}");
        assert_ne!(
            capability(id).target_tier,
            CapabilityTier::Supported,
            "{id}"
        );
    }
}

#[test]
fn text_listing_names_every_capability_and_the_rule() {
    let text = render_capabilities_text();
    for capability in capability_registry() {
        assert!(text.contains(capability.id));
    }
    assert!(text.contains("Supported only when"));
}
