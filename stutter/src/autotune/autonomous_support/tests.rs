use super::*;

const CONTRACT: AutonomousContract = AutonomousContract {
    operation: MutationOperationKind::CpuAffinityProfile,
    workload: WorkloadKind::Game,
    pressure: PressureKind::Scheduler,
    objective: ObjectiveKind::GameRunnableLatency,
    effect_scope: ActionEffectScope::LocalProcessTree,
    prerequisites: &[],
};

fn input(
    operation: MutationOperationKind,
    objective: ObjectiveKind,
) -> AutonomousGateInput<'static> {
    AutonomousGateInput {
        operation,
        objective,
        situation: SituationKind::GameCpuSchedulerPressure,
        effect_scope: ActionEffectScope::LocalProcessTree,
        source: ActionSource::AutotuneRuntime,
        validation_override: None,
    }
}

fn facts(contracts: &[AutonomousContract]) -> GateFacts<'_> {
    GateFacts {
        maturity: ObjectiveMaturity::Supported,
        autonomous_apply_allowed: true,
        capability_supported: true,
        contracts,
    }
}

fn affinity() -> AutonomousGateInput<'static> {
    input(
        MutationOperationKind::CpuAffinityProfile,
        ObjectiveKind::GameRunnableLatency,
    )
}

#[test]
fn only_the_graduated_tuple_is_autonomous() {
    assert_eq!(AUTONOMOUS_CONTRACTS.len(), 1);
    for operation in MutationOperationKind::ALL {
        for objective in ObjectiveKind::ALL {
            let allowed = autonomous_apply_gate(&input(operation, objective)).is_ok();
            assert_eq!(
                allowed,
                operation == MutationOperationKind::CpuAffinityProfile
                    && objective == ObjectiveKind::GameRunnableLatency,
                "{operation:?} / {objective:?}"
            );
        }
    }
}

#[test]
fn the_graduated_tuple_does_not_extend_to_other_workloads_pressures_or_scopes() {
    let granted = affinity();
    assert_eq!(autonomous_apply_gate(&granted), Ok(()));
    for other in [
        SituationKind::GameFocused,
        SituationKind::GameGpuBound,
        SituationKind::CompileCpuBound,
        SituationKind::BrowserCpuPressure,
        SituationKind::CpuPressure,
        SituationKind::Unknown,
    ] {
        let mut gated = granted;
        gated.situation = other;
        assert_eq!(
            autonomous_apply_gate(&gated),
            Err(AutonomousDenial::NotInSupportRegistry),
            "{other:?}"
        );
    }
    let mut wider = granted;
    wider.effect_scope = ActionEffectScope::SystemWide;
    assert_eq!(
        autonomous_apply_gate(&wider),
        Err(AutonomousDenial::NotInSupportRegistry)
    );
}

#[test]
fn registered_contracts_are_never_manual_operations_and_state_their_prerequisites() {
    for contract in AUTONOMOUS_CONTRACTS {
        assert!(!contract.operation.is_manual_only(), "{contract:?}");
        assert!(!contract.prerequisites.is_empty(), "{contract:?}");
    }
}

#[test]
fn a_supported_tuple_passes_only_with_every_condition() {
    let contracts = [CONTRACT];
    assert_eq!(
        autonomous_apply_gate_with(&affinity(), facts(&contracts)),
        Ok(())
    );

    for (mutate, expected) in [
        (
            GateFacts {
                maturity: ObjectiveMaturity::Validating,
                ..facts(&contracts)
            },
            "objective_not_supported",
        ),
        (
            GateFacts {
                autonomous_apply_allowed: false,
                ..facts(&contracts)
            },
            "objective_autonomous_apply_not_allowed",
        ),
        (
            GateFacts {
                capability_supported: false,
                ..facts(&contracts)
            },
            "capability_tier_not_supported",
        ),
        (facts(&[]), "operation_not_in_support_registry"),
    ] {
        let denial = autonomous_apply_gate_with(&affinity(), mutate).unwrap_err();
        assert_eq!(denial.reason_code(), expected);
    }
}

#[test]
fn a_contract_authorises_exactly_its_tuple() {
    let contracts = [CONTRACT];
    let mut other_operation = affinity();
    other_operation.operation = MutationOperationKind::Uclamp;
    let mut other_objective = affinity();
    other_objective.objective = ObjectiveKind::StutterScore;
    let mut other_scope = affinity();
    other_scope.effect_scope = ActionEffectScope::SystemWide;
    let mut other_workload = affinity();
    other_workload.situation = SituationKind::CompileCpuBound;
    let mut other_pressure = affinity();
    other_pressure.situation = SituationKind::GameGpuBound;
    // Another operation is refused for lacking a prepared transaction before the registry is read.
    assert_eq!(
        autonomous_apply_gate_with(&other_operation, facts(&contracts)),
        Err(AutonomousDenial::PreparedTransactionMissing)
    );
    for denied in [other_objective, other_scope, other_workload, other_pressure] {
        assert_eq!(
            autonomous_apply_gate_with(&denied, facts(&contracts)),
            Err(AutonomousDenial::NotInSupportRegistry),
            "{denied:?}"
        );
    }
}

#[test]
fn manual_and_unclassified_operations_are_never_autonomous() {
    let contracts = [CONTRACT];
    for operation in [
        MutationOperationKind::CpuPowerGovernor,
        MutationOperationKind::GpuForcePerformance,
        MutationOperationKind::VmDirtyRatio,
        MutationOperationKind::IrqStorageOrUnknown,
        MutationOperationKind::Unclassified,
    ] {
        let mut gated = input(operation, ObjectiveKind::GameRunnableLatency);
        let grant = ValidationOverride {
            operation,
            objective: ObjectiveKind::GameRunnableLatency,
            campaign_id: "c".to_owned(),
        };
        gated.validation_override = Some(&grant);
        assert_eq!(
            autonomous_apply_gate_with(&gated, facts(&contracts)),
            Err(AutonomousDenial::OperationNotAutonomous),
            "{operation:?}"
        );
    }
}

#[test]
fn a_validation_override_authorises_exactly_one_operation_and_objective() {
    let grant = ValidationOverride {
        operation: MutationOperationKind::CpuAffinityProfile,
        objective: ObjectiveKind::GameRunnableLatency,
        campaign_id: "mutation-campaign-1".to_owned(),
    };
    // GameRunnableLatency is Validating today, which is what an override may exercise.
    let mut exact = affinity();
    exact.validation_override = Some(&grant);
    assert_eq!(autonomous_apply_gate(&exact), Ok(()));

    let mut other_operation = input(
        MutationOperationKind::Uclamp,
        ObjectiveKind::GameRunnableLatency,
    );
    other_operation.validation_override = Some(&grant);
    assert!(autonomous_apply_gate(&other_operation).is_err());

    let mut other_objective = input(
        MutationOperationKind::CpuAffinityProfile,
        ObjectiveKind::StutterScore,
    );
    other_objective.validation_override = Some(&grant);
    assert!(autonomous_apply_gate(&other_objective).is_err());

    // A remote caller never uses an override: for a tuple only an override could authorise (not the
    // registered one) it is refused.
    let uclamp_grant = ValidationOverride {
        operation: MutationOperationKind::Uclamp,
        objective: ObjectiveKind::GameRunnableLatency,
        campaign_id: "mutation-campaign-1".to_owned(),
    };
    let mut remote = input(
        MutationOperationKind::Uclamp,
        ObjectiveKind::GameRunnableLatency,
    );
    remote.source = ActionSource::RemoteAgent;
    remote.validation_override = Some(&uclamp_grant);
    assert!(autonomous_apply_gate(&remote).is_err());
    remote.source = ActionSource::AutotuneRuntime;
    assert_eq!(autonomous_apply_gate(&remote), Ok(()));
}

#[test]
fn an_override_cannot_exercise_an_experimental_objective() {
    let grant = ValidationOverride {
        operation: MutationOperationKind::CpuAffinityProfile,
        objective: ObjectiveKind::StutterScore,
        campaign_id: "c".to_owned(),
    };
    let mut gated = input(
        MutationOperationKind::CpuAffinityProfile,
        ObjectiveKind::StutterScore,
    );
    gated.validation_override = Some(&grant);
    assert_eq!(
        autonomous_apply_gate(&gated),
        Err(AutonomousDenial::ObjectiveNotSupported(
            ObjectiveMaturity::Experimental
        ))
    );
}

#[test]
fn an_operation_without_a_prepared_transaction_is_never_autonomous_even_when_registered() {
    let uclamp = AutonomousContract {
        operation: MutationOperationKind::Uclamp,
        ..CONTRACT
    };
    let contracts = [uclamp];
    let gated = input(
        MutationOperationKind::Uclamp,
        ObjectiveKind::GameRunnableLatency,
    );
    assert_eq!(
        autonomous_apply_gate_with(&gated, facts(&contracts)),
        Err(AutonomousDenial::PreparedTransactionMissing)
    );
}

#[test]
fn a_campaign_holding_the_run_excludes_every_other_operation_even_a_supported_one() {
    let uclamp_grant = ValidationOverride {
        operation: MutationOperationKind::Uclamp,
        objective: ObjectiveKind::GameRunnableLatency,
        campaign_id: "mutation-campaign-2".to_owned(),
    };
    // Without the campaign, the graduated affinity tuple is autonomous...
    assert_eq!(autonomous_apply_gate(&affinity()), Ok(()));
    // ...while the campaign owns the run, only its own operation may be applied.
    let mut during = affinity();
    during.validation_override = Some(&uclamp_grant);
    assert_eq!(
        autonomous_apply_gate(&during),
        Err(AutonomousDenial::OutsideValidationScope)
    );
}
