use super::*;
use crate::autotune::research::{
    ResearchAction, ResearchFactor,
    child_study::{Profile, Stage},
};

fn study(stage: Stage, result: Option<StudyResult>) -> Disposition {
    on(Profile::Cpu, stage, result)
}

fn on(profile: Profile, stage: Stage, result: Option<StudyResult>) -> Disposition {
    Disposition {
        study_sha256: "a".repeat(64),
        profile,
        stage,
        result,
        reason: String::new(),
    }
}

const ADMITTED: crate::autotune::research::lease_inspect::Admission =
    crate::autotune::research::lease_inspect::Admission {
        peer_identified: true,
        facts_consistent: true,
        admitted_in_window: true,
    };

fn full(result: Option<StudyResult>) -> Vec<Disposition> {
    let mut studies = Vec::new();
    // The plan's comparison stages exist for both performance profiles.
    for profile in [Profile::Cpu, Profile::Gpu] {
        for stage in [Stage::Screen, Stage::Pilot, Stage::Confirm, Stage::Holdout] {
            studies.push(on(profile, stage, result));
        }
    }
    studies.extend([
        study(Stage::Action(ResearchAction::CpuAffinityProfile), result),
        study(Stage::Feature(ResearchFactor::NtSync), result),
    ]);
    studies
}

fn run(studies: &[Disposition], restored: bool, health: bool) -> Vec<GateVerdict> {
    gates(&GateInputs {
        raw_files_verified: true,
        loaded_runtime_qualified: true,
        child_studies: studies,
        child_reports_verified: true,
        restoration_verified: restored,
        health_blocked: health,
        telemetry: None,
        replication: None,
        fault_matrix: Err("NoFaultMatrixReceipt"),
        protected_state: Ok(()),
        hardware: Ok(()),
        deployment_attested: false,
        lease: None,
        host: None,
    })
}

fn of<'a>(verdicts: &'a [GateVerdict], name: &str) -> &'a GateVerdict {
    verdicts
        .iter()
        .find(|verdict| verdict.gate == name)
        .unwrap()
}

#[test]
fn there_is_one_verdict_per_gate_and_the_unattestable_ones_never_pass() {
    let verdicts = run(&full(Some(StudyResult::Qualified)), true, false);
    let names: Vec<_> = verdicts.iter().map(|verdict| verdict.gate).collect();
    assert_eq!(
        names,
        ["G0", "G1", "G2", "G3", "G4", "G5", "G6", "G7", "G8", "G9"]
    );
    // A campaign verification cannot attest a host baseline, the lease receipts, resident
    // telemetry, a fault matrix or a deployment: they are named, not inferred from the studies.
    for name in ["G0", "G1", "G3", "G6", "G8"] {
        let verdict = of(&verdicts, name);
        assert!(!verdict.accepted && verdict.blockers.len() == 1, "{name}");
    }
    for name in ["G2", "G4", "G5", "G7", "G9"] {
        let verdict = of(&verdicts, name);
        assert!(verdict.accepted && verdict.blockers.is_empty(), "{name}");
    }
}

#[test]
fn each_gate_names_what_is_missing() {
    // Nothing decided: every study-backed gate is blocked by its own study kind.
    let verdicts = run(&full(None), false, false);
    assert_eq!(of(&verdicts, "G2").blockers, ["ScreenNotQualified"]);
    assert_eq!(of(&verdicts, "G4").blockers, ["ComparisonNotQualified"]);
    assert_eq!(
        of(&verdicts, "G5").blockers,
        ["ActionNotQualified", "RestorationUnverified"]
    );
    assert_eq!(of(&verdicts, "G7").blockers, ["FeatureNotQualified"]);
    assert!(
        of(&verdicts, "G9")
            .blockers
            .contains(&"HoldoutNotQualified")
    );
    // A plan with no such studies says so, rather than passing vacuously.
    let none = run(&[], true, false);
    assert_eq!(of(&none, "G5").blockers, ["NoActionStudy"]);
    assert_eq!(of(&none, "G7").blockers, ["NoFeatureStudy"]);
    assert_eq!(of(&none, "G9").blockers[0], "NoHoldoutStudy");
}

#[test]
fn qualified_studies_do_not_accept_an_action_without_a_proven_restoration() {
    let verdicts = run(&full(Some(StudyResult::Qualified)), false, false);
    assert_eq!(of(&verdicts, "G5").blockers, ["RestorationUnverified"]);
    assert_eq!(of(&verdicts, "G9").blockers, ["G5NotAccepted"]);
}

#[test]
fn a_health_incident_blocks_every_gate_that_needs_a_game() {
    let verdicts = run(&full(Some(StudyResult::Qualified)), true, true);
    for name in ["G2", "G4", "G5", "G7", "G9"] {
        assert_eq!(
            of(&verdicts, name).blockers,
            ["GpuHealthIncident"],
            "{name}"
        );
    }
}

#[test]
fn a_blocked_or_failed_study_is_not_a_qualified_one() {
    for result in [
        StudyResult::Unqualified,
        StudyResult::Failed,
        StudyResult::Blocked,
        StudyResult::DeferredBudget,
    ] {
        let verdicts = run(&full(Some(result)), true, false);
        assert!(!of(&verdicts, "G4").accepted, "{result:?}");
        assert!(!of(&verdicts, "G9").accepted, "{result:?}");
    }
}

#[test]
fn g3_needs_an_eligible_game_bound_receipt_for_every_completed_attempt() {
    let studies = full(Some(StudyResult::Qualified));
    let with = |telemetry: Option<(u32, u32)>, health: bool| {
        let verdicts = gates(&GateInputs {
            raw_files_verified: true,
            loaded_runtime_qualified: true,
            child_studies: &studies,
            child_reports_verified: true,
            restoration_verified: true,
            health_blocked: health,
            telemetry,
            replication: None,
            fault_matrix: Err("NoFaultMatrixReceipt"),
            protected_state: Ok(()),
            hardware: Ok(()),
            deployment_attested: false,
            lease: None,
            host: None,
        });
        of(&verdicts, "G3").clone()
    };
    assert_eq!(with(None, false).blockers, ["NoTelemetryReceipts"]);
    assert_eq!(
        with(Some((7, 8)), false).blockers,
        ["FrameSourceNotEligible"]
    );
    assert_eq!(
        with(Some((0, 8)), false).blockers,
        ["FrameSourceNotEligible"]
    );
    assert!(with(Some((8, 8)), false).accepted);
    assert_eq!(with(Some((8, 8)), true).blockers, ["GpuHealthIncident"]);
}

#[test]
fn a_plan_missing_a_performance_profile_cannot_accept_the_gates_that_need_both() {
    // Only the CPU profile's comparison stages exist: the GPU comparisons the plan asks for do not.
    let mut cpu_only = full(Some(StudyResult::Qualified));
    cpu_only.retain(|study| study.profile != Profile::Gpu);
    let verdicts = run(&cpu_only, true, false);
    for name in ["G2", "G4", "G9"] {
        assert!(
            of(&verdicts, name)
                .blockers
                .contains(&"MissingProfileStudy"),
            "{name}: {:?}",
            of(&verdicts, name).blockers
        );
    }
}

#[test]
fn g8_is_accepted_only_on_an_attested_deployment_and_needs_no_game() {
    let studies = full(Some(StudyResult::Qualified));
    let with = |attested: bool, health: bool| {
        let verdicts = gates(&GateInputs {
            raw_files_verified: true,
            loaded_runtime_qualified: true,
            child_studies: &studies,
            child_reports_verified: true,
            restoration_verified: true,
            health_blocked: health,
            telemetry: None,
            replication: None,
            fault_matrix: Err("NoFaultMatrixReceipt"),
            protected_state: Ok(()),
            hardware: Ok(()),
            deployment_attested: attested,
            lease: None,
            host: None,
        });
        of(&verdicts, "G8").clone()
    };
    assert_eq!(with(false, false).blockers, ["DeploymentNotAttested"]);
    assert!(with(true, false).accepted);
    // A GPU incident is about running a game; what is deployed is unaffected by it.
    assert!(with(true, true).accepted);
}

#[test]
fn g1_stands_on_a_lease_that_holds_together_ended_and_belongs_to_this_campaign() {
    let studies = full(Some(StudyResult::Qualified));
    let with = |lease: Option<LeaseFacts>| {
        let verdicts = gates(&GateInputs {
            raw_files_verified: true,
            loaded_runtime_qualified: true,
            child_studies: &studies,
            child_reports_verified: true,
            restoration_verified: true,
            health_blocked: false,
            telemetry: None,
            replication: None,
            fault_matrix: Err("NoFaultMatrixReceipt"),
            protected_state: Ok(()),
            hardware: Ok(()),
            deployment_attested: false,
            lease,
            host: None,
        });
        of(&verdicts, "G1").clone()
    };
    let good = LeaseFacts {
        chain_valid: true,
        transitions_valid: true,
        header_matches_intent: true,
        terminal: true,
        for_this_campaign: true,
        owner_live: Some(false),
        admission: ADMITTED,
    };
    assert_eq!(with(None).blockers, ["LeaseNotInspected"]);
    assert!(with(Some(good)).accepted);
    assert_eq!(
        with(Some(LeaseFacts {
            chain_valid: false,
            ..good
        }))
        .blockers,
        ["LeaseChainBroken"]
    );
    assert_eq!(
        with(Some(LeaseFacts {
            terminal: false,
            ..good
        }))
        .blockers,
        ["LeaseNotSettled"]
    );
    assert_eq!(
        with(Some(LeaseFacts {
            for_this_campaign: false,
            owner_live: Some(false),
            admission: ADMITTED,
            ..good
        }))
        .blockers,
        ["LeaseForAnotherCampaign"]
    );
    let broken = with(Some(LeaseFacts {
        chain_valid: false,
        transitions_valid: false,
        header_matches_intent: false,
        terminal: false,
        for_this_campaign: false,
        owner_live: Some(false),
        admission: ADMITTED,
    }));
    assert_eq!(broken.blockers.len(), 5);
}

#[test]
fn g0_stands_on_a_host_that_has_not_moved_from_what_the_campaign_froze() {
    let studies = full(Some(StudyResult::Qualified));
    let with = |host: Option<HostFacts>| {
        let verdicts = gates(&GateInputs {
            raw_files_verified: true,
            loaded_runtime_qualified: true,
            child_studies: &studies,
            child_reports_verified: true,
            restoration_verified: true,
            health_blocked: false,
            telemetry: None,
            replication: None,
            fault_matrix: Err("NoFaultMatrixReceipt"),
            protected_state: Ok(()),
            hardware: Ok(()),
            deployment_attested: false,
            lease: None,
            host,
        });
        of(&verdicts, "G0").clone()
    };
    let steady = HostFacts {
        same_boot: true,
        build_id_matches: true,
        game_executable_matches: true,
        installation_settled: true,
        proton_identities_match: true,
        steam_client_matches: true,
        entitlement_verified: true,
    };
    assert_eq!(with(None).blockers, ["HostNotInspected"]);
    assert!(with(Some(steady)).accepted);
    // The Proton builds, the Steam client and the launch record are each their own blocker.
    for (moved, blocker) in [
        (
            HostFacts {
                proton_identities_match: false,
                ..steady
            },
            "ProtonIdentityChanged",
        ),
        (
            HostFacts {
                steam_client_matches: false,
                ..steady
            },
            "SteamClientChanged",
        ),
        (
            HostFacts {
                entitlement_verified: false,
                ..steady
            },
            "EntitlementNotVerified",
        ),
    ] {
        assert_eq!(with(Some(moved)).blockers, [blocker]);
    }
    assert_eq!(
        with(Some(HostFacts {
            same_boot: false,
            ..steady
        }))
        .blockers,
        ["HostRebootedSinceCampaign"]
    );
    assert_eq!(
        with(Some(HostFacts {
            build_id_matches: false,
            ..steady
        }))
        .blockers,
        ["GameBuildChanged"]
    );
    assert_eq!(
        with(Some(HostFacts {
            game_executable_matches: false,
            installation_settled: false,
            ..steady
        }))
        .blockers,
        ["GameExecutableChanged", "InstallationNotSettled"]
    );
}

#[test]
fn g9_needs_the_holdout_to_reproduce_its_confirmation_not_merely_to_be_a_sound_run() {
    let studies = full(Some(StudyResult::Qualified));
    let g9 = |replication: Option<bool>| {
        let verdicts = gates(&GateInputs {
            raw_files_verified: true,
            loaded_runtime_qualified: true,
            child_studies: &studies,
            child_reports_verified: true,
            restoration_verified: true,
            health_blocked: false,
            telemetry: Some((8, 8)),
            replication,
            fault_matrix: Err("NoFaultMatrixReceipt"),
            protected_state: Ok(()),
            hardware: Ok(()),
            deployment_attested: true,
            lease: None,
            host: None,
        });
        of(&verdicts, "G9").blockers.clone()
    };
    // A contradicting or inconclusive holdout is a sound measurement that reproduces nothing.
    assert!(g9(Some(false)).contains(&"HoldoutDoesNotReplicate"));
    assert!(!g9(Some(true)).contains(&"HoldoutDoesNotReplicate"));
    // No holdout judged at all is not a replication blocker of its own.
    assert!(!g9(None).contains(&"HoldoutDoesNotReplicate"));
}

#[test]
fn g6_is_accepted_only_from_a_physical_matrix_receipt_and_otherwise_says_why() {
    let studies = full(Some(StudyResult::Qualified));
    let g6 = |fault_matrix: Result<(), &'static str>| {
        let verdicts = gates(&GateInputs {
            raw_files_verified: true,
            loaded_runtime_qualified: true,
            child_studies: &studies,
            child_reports_verified: true,
            restoration_verified: true,
            health_blocked: false,
            telemetry: Some((8, 8)),
            replication: Some(true),
            fault_matrix,
            protected_state: Ok(()),
            hardware: Ok(()),
            deployment_attested: true,
            lease: None,
            host: None,
        });
        let verdict = of(&verdicts, "G6");
        (verdict.accepted, verdict.blockers.clone())
    };
    assert_eq!(g6(Ok(())), (true, vec![]));
    assert_eq!(
        g6(Err("FaultClassOnlyFixtureTested")),
        (false, vec!["FaultClassOnlyFixtureTested"])
    );
    assert_eq!(
        g6(Err("NoFaultMatrixReceipt")),
        (false, vec!["NoFaultMatrixReceipt"])
    );
}

#[test]
fn g1_names_each_part_of_the_admission_it_cannot_show() {
    use crate::autotune::research::lease_inspect::Admission;
    let studies = full(Some(StudyResult::Qualified));
    let g1 = |admission: Admission| {
        let verdicts = gates(&GateInputs {
            raw_files_verified: true,
            loaded_runtime_qualified: true,
            child_studies: &studies,
            child_reports_verified: true,
            restoration_verified: true,
            health_blocked: false,
            telemetry: Some((8, 8)),
            replication: Some(true),
            fault_matrix: Err("NoFaultMatrixReceipt"),
            protected_state: Ok(()),
            hardware: Ok(()),
            deployment_attested: true,
            lease: Some(LeaseFacts {
                chain_valid: true,
                transitions_valid: true,
                header_matches_intent: true,
                terminal: true,
                for_this_campaign: true,
                owner_live: Some(false),
                admission,
            }),
            host: None,
        });
        of(&verdicts, "G1").blockers.clone()
    };
    assert!(g1(ADMITTED).is_empty());
    assert_eq!(
        g1(Admission {
            peer_identified: false,
            ..ADMITTED
        }),
        ["LeaseAdmissionPeerUnidentified"]
    );
    assert_eq!(
        g1(Admission {
            facts_consistent: false,
            admitted_in_window: false,
            ..ADMITTED
        }),
        ["LeaseFactsInconsistent", "LeaseAdmittedOutsideIntentWindow"]
    );
}

#[test]
fn g2_needs_the_players_original_data_to_be_intact() {
    let studies = full(Some(StudyResult::Qualified));
    let g2 = |protected_state: Result<(), &'static str>| {
        let verdicts = gates(&GateInputs {
            raw_files_verified: true,
            loaded_runtime_qualified: true,
            child_studies: &studies,
            child_reports_verified: true,
            restoration_verified: true,
            health_blocked: false,
            telemetry: Some((8, 8)),
            replication: Some(true),
            fault_matrix: Err("NoFaultMatrixReceipt"),
            protected_state,
            hardware: Ok(()),
            deployment_attested: true,
            lease: None,
            host: None,
        });
        of(&verdicts, "G2").blockers.clone()
    };
    assert!(g2(Ok(())).is_empty());
    // Changed, never recorded and unreadable each block; none is read as unchanged.
    for reason in [
        "ProtectedStateChanged",
        "ProtectedStateNotRecorded",
        "ProtectedStateUnreadable",
        "ProtectedStateTooLarge",
    ] {
        assert_eq!(g2(Err(reason)), [reason]);
    }
}

#[test]
fn g0_also_needs_the_same_kernel_cpu_topology_and_gpu_as_before_the_campaign() {
    let studies = full(Some(StudyResult::Qualified));
    let g0 = |hardware: Result<(), &'static str>| {
        let verdicts = gates(&GateInputs {
            raw_files_verified: true,
            loaded_runtime_qualified: true,
            child_studies: &studies,
            child_reports_verified: true,
            restoration_verified: true,
            health_blocked: false,
            telemetry: Some((8, 8)),
            replication: Some(true),
            fault_matrix: Err("NoFaultMatrixReceipt"),
            protected_state: Ok(()),
            hardware,
            deployment_attested: true,
            lease: None,
            host: Some(HostFacts {
                same_boot: true,
                build_id_matches: true,
                game_executable_matches: true,
                installation_settled: true,
                proton_identities_match: true,
                steam_client_matches: true,
                entitlement_verified: true,
            }),
        });
        let verdict = of(&verdicts, "G0");
        (verdict.accepted, verdict.blockers.clone())
    };
    assert_eq!(g0(Ok(())), (true, vec![]));
    for reason in [
        "HostKernelChanged",
        "HostCpuTopologyChanged",
        "HostGpuChanged",
        "HostHardwareNotRecorded",
        "HostHardwareUnreadable",
    ] {
        assert_eq!(g0(Err(reason)), (false, vec![reason]));
    }
}
