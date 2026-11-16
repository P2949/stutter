use super::*;
use crate::autotune::research::{
    ResearchAction,
    action_child::{ActionChildScope, ChildTarget},
    action_gate::ActionGate,
    child_study::{Stage, plan},
};

fn game() -> ChildTarget {
    ChildTarget {
        pid: stutter_core::ids::Pid::new(4242),
        start_ticks: 99,
        uid: crate::syscall::geteuid(),
    }
}

/// What the controller's manager holds while the permitted CPU action runs on the frozen process.
fn own_holding() -> Vec<crate::autotune::research::action_gate::Holding> {
    vec![crate::autotune::research::action_gate::Holding {
        experiment_id: stutter_core::ids::ExperimentId::new("exp-1"),
        action: Some(ResearchAction::CpuAffinityProfile),
        pid: Some(game().pid),
    }]
}

fn setup(
    study_of: fn(&Stage) -> bool,
) -> (
    tempfile::TempDir,
    DriverStudyAttempts,
    ChildStudy,
    SharedGate,
    ResearchIntentV1,
) {
    setup_with(study_of, Some(crate::autotune::research::Treatment::On))
}

fn setup_with(
    study_of: fn(&Stage) -> bool,
    treatment: Option<crate::autotune::research::Treatment>,
) -> (
    tempfile::TempDir,
    DriverStudyAttempts,
    ChildStudy,
    SharedGate,
    ResearchIntentV1,
) {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut parent = parent();
    parent.actions = vec![ResearchAction::CpuAffinityProfile];
    let study = plan(&parent)
        .unwrap()
        .into_iter()
        .find(|s| study_of(&s.stage))
        .unwrap();
    let gate: SharedGate = std::sync::Arc::new(std::sync::Mutex::new(
        ActionGate::unarmed(Box::new(|| Some(0))).with_state_probe(Box::new(|_| {
            Some(crate::autotune::research::action_gate::KernelState {
                cpus_allowed: "0-15".into(),
                cgroup: "0::/app.slice".into(),
                ..Default::default()
            })
        })),
    ));
    let mut host = Host::finishing();
    host.process = Some(game());
    host.treatment = treatment;
    let mut now = parent.issued_unix_seconds + 1;
    let attempts = DriverStudyAttempts::new(
        File::open(root.path()).unwrap(),
        parent.clone(),
        Box::new(host),
        Box::new(move || {
            now += 1;
            now
        }),
        Box::new(|| Ok(())),
        Box::new(|_| std::path::PathBuf::from("/nonexistent")),
        Box::new(|study| Ok(inputs_for(study))),
    )
    .with_action_gate(gate.clone());
    (root, attempts, study, gate, parent)
}

type SharedGate = crate::autotune::research::action_gate::SharedActionGate;

#[test]
fn an_action_study_arms_the_gate_on_the_sighted_process_and_qualifies_only_on_a_restored_action() {
    let (root, mut runner, study, gate, parent) = setup(|stage| matches!(stage, Stage::Action(_)));
    assert!(!gate.lock().unwrap().is_armed());
    // One tick: the host has sighted the game, so the gate is armed for exactly this action.
    assert_eq!(runner.poll(&study, 100).unwrap(), None);
    assert!(gate.lock().unwrap().is_armed());
    // The normal controller applies and then rolls the action back, the worker holds no token.
    let now = parent.issued_unix_seconds + 5;
    assert!(
        gate.lock()
            .unwrap()
            .permit(Some(ResearchAction::CpuAffinityProfile), game(), now)
    );
    gate.lock()
        .unwrap()
        .observe_holdings(true, 0, own_holding());
    gate.lock().unwrap().observe_runtime(false, 0);
    let outcome = run_to_end(&mut runner, &study);
    assert_eq!(outcome.stage_evidence, Some(true));
    assert!(
        !gate.lock().unwrap().is_armed(),
        "disarmed when the study ends"
    );
    // The proven restoration is recorded beside the study for an independent verification.
    let receipts = root
        .path()
        .join("studies")
        .join(&study.study_sha256[..16])
        .join("action-receipts.json");
    let recorded: serde_json::Value =
        serde_json::from_slice(&std::fs::read(receipts).unwrap()).unwrap();
    assert_eq!(recorded[0]["action"], "CpuAffinityProfile");
    assert_eq!(recorded[0]["original"], recorded[0]["restored"]);
}

#[test]
fn an_action_study_where_nothing_was_applied_is_not_evidence_and_other_studies_carry_none() {
    let (_root, mut runner, study, gate, _) = setup(|stage| matches!(stage, Stage::Action(_)));
    let outcome = run_to_end(&mut runner, &study);
    assert_eq!(
        outcome.stage_evidence,
        Some(false),
        "no action was ever applied"
    );
    assert!(!gate.lock().unwrap().is_armed());
    // A screen never arms the gate and carries no stage evidence.
    let (_root2, mut screen_runner, screen, gate2, _) =
        setup(|stage| matches!(stage, Stage::Screen));
    let outcome = run_to_end(&mut screen_runner, &screen);
    assert_eq!(outcome.stage_evidence, None);
    assert!(!gate2.lock().unwrap().is_armed());
}

#[test]
fn a_gate_that_still_holds_an_outstanding_action_is_not_clean_evidence() {
    let (_root, mut runner, study, gate, parent) = setup(|stage| matches!(stage, Stage::Action(_)));
    runner.poll(&study, 100).unwrap();
    let now = parent.issued_unix_seconds + 5;
    assert!(
        gate.lock()
            .unwrap()
            .permit(Some(ResearchAction::CpuAffinityProfile), game(), now)
    );
    gate.lock()
        .unwrap()
        .observe_holdings(true, 0, own_holding());
    // The study ends with the action still applied (kept or running): not a proven restoration.
    gate.lock()
        .unwrap()
        .observe_holdings(false, 1, own_holding());
    let outcome = run_to_end(&mut runner, &study);
    assert_eq!(outcome.stage_evidence, Some(false));
    let _ = ActionChildScope::new([], game(), 0);
}

#[test]
fn an_action_study_is_one_build_off_versus_on_and_the_gate_permits_only_on_attempts() {
    use crate::autotune::research::Treatment;
    let permits = |treatment| {
        let (_root, mut runner, study, gate, parent) =
            setup_with(|stage| matches!(stage, Stage::Action(_)), Some(treatment));
        assert_eq!(runner.poll(&study, 100).unwrap(), None);
        assert!(gate.lock().unwrap().is_armed());
        let now = parent.issued_unix_seconds + 5;
        let mut gate = gate.lock().unwrap();
        gate.permit(Some(ResearchAction::CpuAffinityProfile), game(), now)
    };
    assert!(
        permits(Treatment::On),
        "an attempt treated On may be acted on"
    );
    assert!(
        !permits(Treatment::Off),
        "an attempt treated Off applies nothing"
    );
}

#[test]
fn an_action_study_directory_holds_a_one_build_treatment_intent() {
    let (root, mut runner, study, _gate, parent) = setup(|stage| matches!(stage, Stage::Action(_)));
    assert_eq!(runner.poll(&study, 100).unwrap(), None);
    let intent: ResearchIntentV1 = serde_json::from_slice(
        &std::fs::read(
            root.path()
                .join("studies")
                .join(&study.study_sha256[..16])
                .join("intent.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(intent.tools.len(), 1, "one Proton build");
    assert_eq!(intent.treatment, study.treatment_spec());
    assert!(intent.treatment.is_some());
    assert_eq!(intent.max_attempts, study.min_attempts);
    assert!(parent.tools.len() >= 2, "the campaign itself has several");
}

#[test]
fn a_finished_treatment_study_reconstructs_and_is_derived_from_its_campaign() {
    use crate::autotune::research::child_verify::{verify_child, verify_children};
    let (root, mut runner, study, _gate, parent) = setup(|stage| matches!(stage, Stage::Action(_)));
    run_to_end(&mut runner, &study);
    let uid = crate::syscall::geteuid();
    let lookup = |_: ProtonSource| std::path::PathBuf::from("/nonexistent");
    let name = study.study_sha256[..16].to_owned();
    let dir = File::open(root.path().join("studies").join(&name)).unwrap();
    // The child itself reconstructs and is derived from its campaign's plan.
    let (_raw, report, _runtime) = verify_child(&dir, &name, uid, &parent, &lookup).unwrap();
    // The funnel verifier also demands the screen its build was selected from: with no screen
    // measured here it refuses the child rather than trust an unreproducible selection.
    let result = verify_children(&File::open(root.path()).unwrap(), uid, &parent, &lookup).unwrap();
    assert!(!result.children[0].reconstruction && result.children[0].problem.is_some());
    // The comparison is within one build: a treatment, not a second Proton.
    assert_eq!(report["treatment"]["action"], "cpu_affinity_profile");
    assert_eq!(report["arms"].as_array().unwrap().len(), 1);
}

#[test]
fn a_failed_or_conflicting_receipt_write_means_the_action_study_cannot_qualify() {
    let (root, mut runner, study, gate, parent) = setup(|stage| matches!(stage, Stage::Action(_)));
    assert_eq!(runner.poll(&study, 100).unwrap(), None);
    // Something else already holds the receipts file with different bytes.
    let file = root
        .path()
        .join("studies")
        .join(&study.study_sha256[..16])
        .join("action-receipts.json");
    std::fs::write(&file, b"[]").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    let now = parent.issued_unix_seconds + 5;
    assert!(
        gate.lock()
            .unwrap()
            .permit(Some(ResearchAction::CpuAffinityProfile), game(), now)
    );
    gate.lock()
        .unwrap()
        .observe_holdings(true, 0, own_holding());
    gate.lock().unwrap().observe_runtime(false, 0);
    let outcome = run_to_end(&mut runner, &study);
    assert_eq!(
        outcome.stage_evidence,
        Some(false),
        "an action without durable receipts is not qualified"
    );
}
