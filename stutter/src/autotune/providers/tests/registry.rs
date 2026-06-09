use super::*;

#[test]
fn registry_includes_safe_and_suggest_first_provider_families() {
    let registry = CandidateProviderRegistry::default_for_policy(
        &policy_with_system_wide_suggestions(DaemonMode::Suggest),
    );
    let families = registry.families();

    assert!(families.contains(&"cpu_affinity_profile"));
    assert!(families.contains(&"nice"));
    assert!(families.contains(&"ionice"));
    assert!(families.contains(&"uclamp"));
    assert!(families.contains(&"cgroup_placement"));
    assert!(families.contains(&"irq_affinity"));
    assert!(families.contains(&"cpu_power"));
    assert!(families.contains(&"gpu_power"));
    assert!(families.contains(&"vm_knob"));
}

#[test]
fn registered_providers_expose_complete_policy_metadata() {
    let registry = CandidateProviderRegistry::default_for_policy(
        &policy_with_system_wide_suggestions(DaemonMode::Suggest),
    );

    for metadata in registry.metadata() {
        assert!(!metadata.family.is_empty());
        assert!(!metadata.description.trim().is_empty());
        assert_ne!(
            metadata.rollback_requirement,
            RollbackRequirement::Unavailable
        );
        assert!(
            !metadata.capability_requirements.is_empty(),
            "{} must document capability requirements",
            metadata.family
        );
        assert_ne!(
            metadata.conflict_group,
            ActionConflictGroup::None,
            "{} must declare a conflict group",
            metadata.family
        );
        assert!(!metadata.cooldown_key.is_empty());
        assert!(
            !metadata.policy_coverage.is_empty(),
            "{} must document policy gates",
            metadata.family
        );

        match metadata.safety_class {
            SafetyClass::ReversibleLowRisk
            | SafetyClass::ReversibleMediumRisk
            | SafetyClass::HighRisk => {}
            SafetyClass::ObserveOnly => {
                panic!(
                    "{} is an action provider and must not be observe-only",
                    metadata.family
                )
            }
        }

        match metadata.required_mode {
            DaemonMode::Suggest | DaemonMode::ApplyLowRisk | DaemonMode::ApplyMediumRisk => {}
            DaemonMode::Observe | DaemonMode::ApplyHighRisk => {
                panic!(
                    "{} must declare suggest/apply-low/apply-medium as its required mode",
                    metadata.family
                )
            }
        }

        let objective = format!("{:?}", metadata.objective);
        assert!(!objective.is_empty());
    }
}
