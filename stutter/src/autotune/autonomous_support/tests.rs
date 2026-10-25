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
fn nothing_is_supported_today_so_normal_autonomous_apply_fails_closed() {
    assert!(AUTONOMOUS_CONTRACTS.is_empty());
    for operation in MutationOperationKind::ALL {
        for objective in ObjectiveKind::ALL {
            assert!(
                autonomous_apply_gate(&input(operation, objective)).is_err(),
                "{operation:?} / {objective:?} must not be autonomous"
            );
        }
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
    for denied in [
        other_operation,
        other_objective,
        other_scope,
        other_workload,
        other_pressure,
    ] {
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

    let mut remote = exact;
    remote.source = ActionSource::RemoteAgent;
    assert!(autonomous_apply_gate(&remote).is_err());
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
