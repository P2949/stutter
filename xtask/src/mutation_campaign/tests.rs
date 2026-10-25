use std::{cell::RefCell, collections::BTreeMap};

use stutter::api::autotune::ObjectiveKind;

use super::{
    benefit::{Order, PairResult, RunnableSummaryRecord, Scenario},
    benefit_stage,
    driver::{Next, drive_with, next_stage},
    graph::Stage,
    manifest::MutationManifest,
    model::{Disposition, MutationMode, MutationState, SCHEMA_VERSION},
    runner::{
        BenefitHarness, HarnessError, StageContext, StageOutcome, StageRunner, UnavailableHarness,
    },
    store,
};
use crate::validation_campaign::{
    model::{AttemptOutcome, BlockReason, Boundary, FailureClass, TaskState, TaskStatus},
    state::CampaignDir,
};

const MANIFEST: &str = include_str!("../../../docs/validation/mutation-graduation.toml");
const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("stutter-mutation-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn state(scratch: &Scratch, manifest: &MutationManifest) -> (CampaignDir, MutationState) {
    let dir = store::dir_for(&scratch.0, "mut-test");
    std::fs::create_dir_all(&dir.0).unwrap();
    let section = manifest.operation("cpu_affinity_profile").unwrap();
    let state = MutationState {
        schema_version: SCHEMA_VERSION,
        campaign_id: "mut-test".to_owned(),
        generation: 1,
        mode: MutationMode::Active,
        repository: scratch.0.clone(),
        evidence_base: COMMIT.to_owned(),
        manifest_sha256: "a".repeat(64),
        cargo_lock_sha256: "b".repeat(64),
        xtask_sha256: "c".repeat(64),
        stutter_sha256: "d".repeat(64),
        uid: 1000,
        gid: 1000,
        created_unix_ms: 1,
        operation: "cpu_affinity_profile".to_owned(),
        objective: section.objective.clone(),
        tasks: section
            .stage_list()
            .unwrap()
            .into_iter()
            .map(|stage| (stage.as_str().to_owned(), TaskState::new(stage.as_str())))
            .collect(),
        skipped: BTreeMap::new(),
        decision: None,
        frozen: Default::default(),
    };
    store::save(&dir, &state).unwrap();
    (dir, state)
}

/// Passes every stage in the order requested, recording the sequence; a stage listed in
/// `script` gets its scripted outcome once.
struct Scripted {
    seen: RefCell<Vec<(Stage, u32)>>,
    script: RefCell<Vec<(Stage, StageOutcome)>>,
}

impl Scripted {
    fn new(script: Vec<(Stage, StageOutcome)>) -> Self {
        Self {
            seen: RefCell::new(Vec::new()),
            script: RefCell::new(script),
        }
    }
}

impl StageRunner for Scripted {
    fn run(&self, context: &StageContext<'_>, stage: Stage, attempt: u32) -> StageOutcome {
        self.seen.borrow_mut().push((stage, attempt));
        let mut script = self.script.borrow_mut();
        if let Some(position) = script.iter().position(|(scripted, _)| *scripted == stage) {
            return script.remove(position).1;
        }
        if stage == Stage::TierDecision {
            // The real tier stage derives the decision from the recorded outcomes.
            return super::decision::tier(context, attempt);
        }
        StageOutcome::passed(None, None)
    }
}

fn ok(_: &MutationState) -> Result<(), String> {
    Ok(())
}

fn run(
    scratch: &Scratch,
    runner: &dyn StageRunner,
    max: Option<usize>,
) -> (Boundary, CampaignDir, MutationManifest) {
    let manifest = MutationManifest::parse(MANIFEST).unwrap();
    let (dir, _) = if store::state_path(&store::dir_for(&scratch.0, "mut-test")).is_file() {
        (store::dir_for(&scratch.0, "mut-test"), ())
    } else {
        let (dir, _) = state(scratch, &manifest);
        (dir, ())
    };
    let boundary = drive_with(&scratch.0, &dir, &manifest, runner, max, &ok).unwrap();
    (boundary, dir, manifest)
}

#[test]
fn every_stage_passing_completes_with_a_supported_decision() {
    let scratch = Scratch::new("complete");
    let runner = Scripted::new(Vec::new());
    let (boundary, dir, _) = run(&scratch, &runner, None);
    assert_eq!(boundary, Boundary::Complete);
    let state = store::load(&dir).unwrap();
    assert_eq!(state.mode, MutationMode::Complete);
    assert_eq!(
        state.decision.as_ref().unwrap().disposition,
        Disposition::Supported
    );
    let order: Vec<Stage> = runner
        .seen
        .borrow()
        .iter()
        .map(|(stage, _)| *stage)
        .collect();
    assert_eq!(
        order,
        vec![
            Stage::Preflight,
            Stage::Mechanical,
            Stage::FaultMatrix,
            Stage::SuspendResume,
            Stage::BenefitCalibration,
            Stage::BenefitHoldout,
            Stage::EvidenceIntegrate,
            Stage::TierDecision,
        ]
    );
}

#[test]
fn a_retryable_failure_is_retried_within_the_budget_then_stops() {
    let scratch = Scratch::new("retry");
    let fail =
        || StageOutcome::failed(FailureClass::Retryable, "launch", "the suite did not start");
    let runner = Scripted::new(vec![
        (Stage::Mechanical, fail()),
        (Stage::Mechanical, fail()),
    ]);
    let (boundary, dir, _) = run(&scratch, &runner, None);
    assert_eq!(boundary, Boundary::FailureRequiresSourceChange);
    let state = store::load(&dir).unwrap();
    assert_eq!(
        state.tasks["mechanical_apply_verify_rollback"]
            .attempts
            .len(),
        2
    );
    assert_eq!(state.mode, MutationMode::Active);
    assert!(state.decision.is_none());
}

#[test]
fn a_retry_that_succeeds_completes_and_keeps_every_attempt() {
    let scratch = Scratch::new("retry-ok");
    let runner = Scripted::new(vec![(
        Stage::Mechanical,
        StageOutcome::failed(FailureClass::Retryable, "launch", "the suite did not start"),
    )]);
    let (boundary, dir, _) = run(&scratch, &runner, None);
    assert_eq!(boundary, Boundary::Complete);
    let state = store::load(&dir).unwrap();
    let attempts = &state.tasks["mechanical_apply_verify_rollback"].attempts;
    assert_eq!(attempts.len(), 2);
    assert!(matches!(attempts[0].outcome, AttemptOutcome::Failed { .. }));
    assert!(matches!(attempts[1].outcome, AttemptOutcome::Passed));
}

#[test]
fn a_block_stops_with_the_shared_exit_boundary_and_is_rechecked_later() {
    let scratch = Scratch::new("blocked");
    let runner = Scripted::new(vec![(
        Stage::SuspendResume,
        StageOutcome::blocked(FailureClass::BlockedResource, "the machine cannot suspend"),
    )]);
    let (boundary, dir, _) = run(&scratch, &runner, None);
    assert_eq!(boundary, Boundary::BlockedResource);
    assert_eq!(
        store::load(&dir).unwrap().tasks["suspend_resume"].status,
        TaskStatus::BlockedResource
    );
    // The resource appears: the same campaign continues from the blocked stage.
    let resumed = Scripted::new(Vec::new());
    let manifest = MutationManifest::parse(MANIFEST).unwrap();
    let boundary = drive_with(&scratch.0, &dir, &manifest, &resumed, None, &ok).unwrap();
    assert_eq!(boundary, Boundary::Complete);
    assert_eq!(resumed.seen.borrow()[0].0, Stage::SuspendResume);
}

#[test]
fn a_validation_failure_defers_the_operation_instead_of_forcing_support() {
    let scratch = Scratch::new("deferred");
    let runner = Scripted::new(vec![(
        Stage::BenefitHoldout,
        StageOutcome::Failed {
            class: FailureClass::ValidationFailure,
            stage: "holdout".to_owned(),
            summary: "a false keep".to_owned(),
            artifacts: None,
        },
    )]);
    let (boundary, dir, _) = run(&scratch, &runner, None);
    assert_eq!(boundary, Boundary::Complete);
    let state = store::load(&dir).unwrap();
    let decision = state.decision.unwrap();
    assert_eq!(decision.disposition, Disposition::DeferredEvidence);
    assert!(
        decision
            .reasons
            .iter()
            .any(|r| r.contains("benefit_holdout"))
    );
    // The stages after the failed one never ran.
    let seen: Vec<Stage> = runner.seen.borrow().iter().map(|(s, _)| *s).collect();
    assert!(!seen.contains(&Stage::EvidenceIntegrate));
}

#[test]
fn max_stages_stops_early_and_next_stage_reports_progress() {
    let scratch = Scratch::new("max");
    let runner = Scripted::new(Vec::new());
    let (boundary, dir, _) = run(&scratch, &runner, Some(2));
    assert_eq!(boundary, Boundary::Idle);
    let state = store::load(&dir).unwrap();
    assert_eq!(state.mode, MutationMode::Active);
    assert_eq!(
        next_stage(&state, 2),
        Next::Stage(Stage::FaultMatrix),
        "{:?}",
        state.tasks.keys().collect::<Vec<_>>()
    );
}

#[test]
fn a_changed_source_invalidates_the_campaign() {
    let scratch = Scratch::new("invalidate");
    let manifest = MutationManifest::parse(MANIFEST).unwrap();
    let (dir, _) = state(&scratch, &manifest);
    let runner = Scripted::new(Vec::new());
    let boundary = drive_with(&scratch.0, &dir, &manifest, &runner, None, &|_| {
        Err("HEAD moved".to_owned())
    })
    .unwrap();
    assert_eq!(boundary, Boundary::FailureRequiresSourceChange);
    assert_eq!(store::load(&dir).unwrap().mode, MutationMode::Invalidated);
    assert!(runner.seen.borrow().is_empty());
    // An invalidated campaign never continues.
    assert!(drive_with(&scratch.0, &dir, &manifest, &runner, None, &ok).is_err());
}

fn pair(scenario: Scenario, index: usize, candidate_p99: u64) -> PairResult {
    PairResult {
        scenario,
        pair: index,
        order: Order::BaselineFirst,
        baseline: RunnableSummaryRecord {
            p99_ns: 10_000_000,
            samples: 2_000,
            over_5ms: 20,
        },
        candidate: RunnableSummaryRecord {
            p99_ns: candidate_p99,
            samples: 2_000,
            over_5ms: 20,
        },
        placement_observed: true,
        guardrails_ok: true,
        manager_kept: false,
    }
}

/// Measures from a table; counts the pairs it was asked for.
struct Table {
    asked: RefCell<usize>,
    candidate: fn(Scenario) -> u64,
}

impl BenefitHarness for Table {
    fn measure(
        &self,
        _: &StageContext<'_>,
        _: ObjectiveKind,
        scenario: Scenario,
        pair_index: usize,
        _: Order,
    ) -> Result<PairResult, HarnessError> {
        *self.asked.borrow_mut() += 1;
        Ok(pair(scenario, pair_index, (self.candidate)(scenario)))
    }
}

fn clear(scenario: Scenario) -> u64 {
    match scenario {
        Scenario::Beneficial => 7_000_000,
        Scenario::Neutral => 10_050_000,
        Scenario::Regression => 12_500_000,
    }
}

fn context_for<'a>(
    scratch: &'a Scratch,
    dir: &'a CampaignDir,
    state: &'a MutationState,
    manifest: &'a MutationManifest,
    binaries: &'a crate::validation_campaign::repository::Binaries,
) -> StageContext<'a> {
    StageContext {
        repository: &scratch.0,
        dir,
        state,
        manifest,
        operation: manifest.operation("cpu_affinity_profile").unwrap(),
        binaries,
    }
}

fn binaries() -> crate::validation_campaign::repository::Binaries {
    crate::validation_campaign::repository::Binaries {
        stutter: "stutter".into(),
        xtask: "xtask".into(),
    }
}

#[test]
fn clear_data_calibrates_at_the_initial_pair_count_and_the_holdout_is_final() {
    let scratch = Scratch::new("benefit");
    let manifest = MutationManifest::parse(MANIFEST).unwrap();
    let (dir, st) = state(&scratch, &manifest);
    let binaries = binaries();
    let context = context_for(&scratch, &dir, &st, &manifest, &binaries);
    let table = Table {
        asked: RefCell::new(0),
        candidate: clear,
    };
    let outcome = benefit_stage::calibrate(&context, 1, &table);
    assert!(
        matches!(outcome, StageOutcome::Passed { .. }),
        "{outcome:?}"
    );
    assert_eq!(*table.asked.borrow(), 3 * manifest.benefit.initial_pairs);

    let holdout = benefit_stage::holdout(&context, 1);
    assert!(
        matches!(holdout, StageOutcome::Passed { .. }),
        "{holdout:?}"
    );
    // Re-running never re-evaluates: the stored verdict is returned and no pair is added.
    let before =
        benefit_stage::read_pairs(&benefit_stage::benefit_dir(&context).join("pairs.jsonl"))
            .unwrap()
            .len();
    assert!(matches!(
        benefit_stage::holdout(&context, 2),
        StageOutcome::Passed { .. }
    ));
    let after =
        benefit_stage::read_pairs(&benefit_stage::benefit_dir(&context).join("pairs.jsonl"))
            .unwrap()
            .len();
    assert_eq!(before, after);
}

fn hopeless(scenario: Scenario) -> u64 {
    match scenario {
        // A neutral pair that looks like a 40 % win: no threshold in the grid rejects it while
        // keeping the beneficial ones.
        Scenario::Neutral => 6_000_000,
        _ => clear(scenario),
    }
}

#[test]
fn an_unseparable_scenario_tops_up_to_the_cap_then_fails_validation() {
    let scratch = Scratch::new("topup");
    let manifest = MutationManifest::parse(MANIFEST).unwrap();
    let (dir, st) = state(&scratch, &manifest);
    let binaries = binaries();
    let context = context_for(&scratch, &dir, &st, &manifest, &binaries);
    let table = Table {
        asked: RefCell::new(0),
        candidate: hopeless,
    };
    let outcome = benefit_stage::calibrate(&context, 1, &table);
    match outcome {
        StageOutcome::Failed { class, .. } => assert_eq!(class, FailureClass::ValidationFailure),
        other => panic!("{other:?}"),
    }
    assert_eq!(*table.asked.borrow(), 3 * manifest.benefit.max_pairs);
}

#[test]
fn the_holdout_without_frozen_thresholds_is_a_block_not_a_pass() {
    let scratch = Scratch::new("no-calibration");
    let manifest = MutationManifest::parse(MANIFEST).unwrap();
    let (dir, st) = state(&scratch, &manifest);
    let binaries = binaries();
    let context = context_for(&scratch, &dir, &st, &manifest, &binaries);
    assert!(matches!(
        benefit_stage::holdout(&context, 1),
        StageOutcome::Blocked(BlockReason { .. })
    ));
}

#[test]
fn a_build_without_a_harness_blocks_honestly() {
    let scratch = Scratch::new("no-harness");
    let manifest = MutationManifest::parse(MANIFEST).unwrap();
    let (dir, st) = state(&scratch, &manifest);
    let binaries = binaries();
    let context = context_for(&scratch, &dir, &st, &manifest, &binaries);
    assert!(matches!(
        benefit_stage::calibrate(&context, 1, &UnavailableHarness),
        StageOutcome::Blocked(_)
    ));
}
