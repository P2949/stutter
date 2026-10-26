use super::{super::*, support::*};
use crate::autotune::{
    kept::KeptCandidateState,
    kept_ledger::{KeptLedgerEntry, kept_ledger_path, read_kept_ledger, record_kept},
};

fn kept_state(name: &str) -> KeptCandidateState {
    let candidate = CandidateAction::fake(
        ActionId::new(name.to_owned()),
        SafetyClass::ReversibleLowRisk,
    );
    KeptCandidateState::new(
        ExperimentId::new(format!("experiment-{name}")),
        candidate,
        score(1_000),
        score(500),
        fake_rollback(),
        10,
        "kept",
    )
}

fn ledger_entry(kept: &KeptCandidateState) -> KeptLedgerEntry {
    KeptLedgerEntry {
        experiment_id: kept.experiment_id.clone(),
        action_id: kept.candidate.action_id(),
        candidate: kept.candidate.profile_name().to_owned(),
        rollback_token: kept.rollback.clone(),
        mode: None,
        safety_class: None,
        kept_unix_nanos: 10,
    }
}

#[test]
fn keeping_an_experiment_makes_its_token_durable_before_the_next_experiment() {
    let journal_path = temp_journal_path("kept-ledger");
    let mut manager = LiveExperimentManager::new();
    manager.set_current_for_tests(live_experiment());
    let mut controller_state = ControllerRuntimeState::default();
    let mut profile = ActiveProfileState::default();
    let mut executor = FakeLiveExecutor::default();

    manager
        .keep_current_experiment_with_executor(
            &input(journal_path.clone()),
            &mut controller_state,
            &mut profile,
            &observation(500, 400),
            "improved",
            &mut executor,
        )
        .unwrap();

    assert_eq!(profile.kept_action_count(), 1);
    let ledger = read_kept_ledger(&kept_ledger_path(&journal_path)).unwrap();
    assert_eq!(ledger.entries.len(), 1);
    assert_eq!(ledger.entries[0].rollback_token, fake_rollback());
}

#[test]
fn releasing_kept_actions_restores_each_and_retires_its_ledger_entry() {
    let journal_path = temp_journal_path("kept-release");
    let mut profile = ActiveProfileState::default();
    let kept = kept_state("fake-kept");
    record_kept(&kept_ledger_path(&journal_path), ledger_entry(&kept)).unwrap();
    profile
        .record_kept_candidate(
            kept,
            crate::autotune::comparison::ExperimentResult::Improved {
                improvement_percent: 50.0,
            },
        )
        .unwrap();
    let mut executor = FakeLiveExecutor::default();

    let release = LiveExperimentManager::release_kept_actions_with_executor(
        &input(journal_path.clone()),
        &mut profile,
        &observation(500, 400),
        "test",
        &mut executor,
    );

    assert_eq!((release.restored, release.failed), (1, 0));
    assert_eq!(executor.rollback_calls, 1);
    assert_eq!(profile.kept_action_count(), 0);
    assert!(
        read_kept_ledger(&kept_ledger_path(&journal_path))
            .unwrap()
            .entries
            .is_empty()
    );
}

#[test]
fn a_kept_action_that_cannot_be_restored_stays_kept_and_in_the_ledger() {
    let journal_path = temp_journal_path("kept-release-fails");
    let mut profile = ActiveProfileState::default();
    let kept = kept_state("fake-kept-fails");
    record_kept(&kept_ledger_path(&journal_path), ledger_entry(&kept)).unwrap();
    profile
        .record_kept_candidate(
            kept,
            crate::autotune::comparison::ExperimentResult::Improved {
                improvement_percent: 50.0,
            },
        )
        .unwrap();
    let mut executor = FakeLiveExecutor {
        fail_rollback: true,
        ..FakeLiveExecutor::default()
    };

    let release = LiveExperimentManager::release_kept_actions_with_executor(
        &input(journal_path.clone()),
        &mut profile,
        &observation(500, 400),
        "test",
        &mut executor,
    );

    assert_eq!((release.restored, release.failed), (0, 1));
    assert_eq!(profile.kept_action_count(), 1);
    assert_eq!(
        read_kept_ledger(&kept_ledger_path(&journal_path))
            .unwrap()
            .entries
            .len(),
        1
    );
}

#[test]
fn a_cooldown_decision_never_clears_a_fault() {
    let journal_path = temp_journal_path("cooldown-keeps-fault");
    let mut manager = LiveExperimentManager::new();
    let mut controller_state = ControllerRuntimeState {
        phase: ControllerPhase::Faulted,
        ..ControllerRuntimeState::default()
    };
    let mut profile = ActiveProfileState::default();
    let mut executor = FakeLiveExecutor::default();

    manager
        .apply_decision_side_effects_with_executor(
            input(journal_path),
            LiveExperimentRuntimeState {
                controller_state: &mut controller_state,
                active_profile_state: &mut profile,
            },
            &observation(1_000, 400),
            &AutotuneDecision::EnterCooldown {
                duration: std::time::Duration::from_secs(60),
                reason: "cooldown".to_owned(),
            },
            "cooldown",
            &mut executor,
        )
        .unwrap();

    assert_eq!(controller_state.phase, ControllerPhase::Faulted);
    assert!(controller_state.cooldown_until_unix_nanos.is_some());
}
