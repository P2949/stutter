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

fn setup(
    study_of: fn(&Stage) -> bool,
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
    let gate: SharedGate =
        std::sync::Arc::new(std::sync::Mutex::new(ActionGate::unarmed(Box::new(|| {
            Some(0)
        }))));
    let mut host = Host::finishing();
    host.process = Some(game());
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
    let (_root, mut runner, study, gate, parent) = setup(|stage| matches!(stage, Stage::Action(_)));
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
    gate.lock().unwrap().observe_runtime(true, 0);
    gate.lock().unwrap().observe_runtime(false, 0);
    let outcome = run_to_end(&mut runner, &study);
    assert_eq!(outcome.stage_evidence, Some(true));
    assert!(
        !gate.lock().unwrap().is_armed(),
        "disarmed when the study ends"
    );
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
    gate.lock().unwrap().observe_runtime(true, 0);
    // The study ends with the action still applied (kept or running): not a proven restoration.
    gate.lock().unwrap().observe_runtime(false, 1);
    let outcome = run_to_end(&mut runner, &study);
    assert_eq!(outcome.stage_evidence, Some(false));
    let _ = ActionChildScope::new([], game(), 0);
}
