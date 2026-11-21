//! A CP-LONG action study run through the real `DriverStudyAttempts` over the long host: the attempt is a
//! sealed session record instead of a native benchmark result, the study qualifies on the gate's proven
//! restoration, and independent verification replays the session record against the intent's unit.

use std::collections::VecDeque;

use super::*;
use crate::autotune::research::{
    ResearchAction,
    action_child::ChildTarget,
    action_gate::{ActionGate, Holding, KernelState},
    child_study::{Stage, StudyResult, plan},
    long_host::LongAttemptHost,
    long_receipt::{LongProgress, LongSessionDriver, LongSessionRecord, Phase, SessionProcess},
    long_session::LongWorkUnit,
    study_executor::judge,
};

fn game() -> ChildTarget {
    ChildTarget {
        pid: stutter_core::ids::Pid::new(4242),
        start_ticks: 99,
        uid: crate::syscall::geteuid(),
    }
}

fn unit() -> LongWorkUnit {
    LongWorkUnit {
        id: "save-a-route-1".into(),
        duration_seconds: 1200,
        definition_sha256: "a".repeat(64),
    }
}

/// Plays the route to completion on the second poll of every session, as the unit says it must.
struct Route {
    definition: String,
    states: VecDeque<LongProgress>,
}

impl LongSessionDriver for Route {
    fn start(&mut self, _: &LongWorkUnit) -> Result<(), &'static str> {
        self.states = VecDeque::from([
            LongProgress::Running,
            LongProgress::Completed(Box::new(session(&self.definition))),
        ]);
        Ok(())
    }
    fn poll(&mut self) -> LongProgress {
        self.states.pop_front().unwrap_or(LongProgress::Running)
    }
    fn abort(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
}

fn session(definition: &str) -> LongSessionRecord {
    LongSessionRecord {
        unit_id: "save-a-route-1".into(),
        definition_sha256: definition.into(),
        started_unix: 5_000,
        ended_unix: 5_000 + 1200,
        process: SessionProcess {
            pid: game().pid,
            start_ticks: game().start_ticks,
            uid: game().uid,
        },
        phases: vec![Phase {
            name: "route".into(),
            started_unix: 5_000,
            ended_unix: 6_200,
        }],
        settled: true,
    }
}

type Shared = crate::autotune::research::action_gate::SharedActionGate;

fn build(
    definition: &str,
) -> (
    tempfile::TempDir,
    DriverStudyAttempts,
    ChildStudy,
    Shared,
    ResearchIntentV1,
) {
    build_with(Route {
        definition: definition.to_owned(),
        states: VecDeque::new(),
    })
}

fn build_with<D: LongSessionDriver + Send + 'static>(
    driver: D,
) -> (
    tempfile::TempDir,
    DriverStudyAttempts,
    ChildStudy,
    Shared,
    ResearchIntentV1,
) {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut parent = parent();
    parent.actions = vec![ResearchAction::CpuAffinityProfile];
    parent.max_active_seconds = 3600;
    parent.max_evidence_bytes = 1024 * 1024;
    parent.long_attempt_seconds = Some(1800);
    parent.long_work_unit = Some(unit());
    parent.validate().unwrap();
    let study = plan(&parent)
        .unwrap()
        .into_iter()
        .find(|s| matches!(s.stage, Stage::Action(_)))
        .unwrap();
    let gate: Shared = std::sync::Arc::new(std::sync::Mutex::new(
        ActionGate::unarmed(Box::new(|| Some(0))).with_state_probe(Box::new(|_| {
            Some(KernelState {
                cpus_allowed: "0-15".into(),
                cgroup: "0::/app.slice".into(),
                ..Default::default()
            })
        })),
    ));
    let mut inner = Host::finishing();
    inner.process = Some(game());
    inner.treatment = Some(crate::autotune::research::Treatment::On);
    let host = LongAttemptHost::new(inner, driver);
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

fn holding() -> Vec<Holding> {
    vec![Holding {
        experiment_id: stutter_core::ids::ExperimentId::new("exp-1"),
        action: Some(ResearchAction::CpuAffinityProfile),
        pid: Some(game().pid),
    }]
}

/// The normal controller applies the action once and restores it, as the gate observes.
fn act(gate: &Shared, parent: &ResearchIntentV1) {
    let now = parent.issued_unix_seconds + 5;
    assert!(
        gate.lock()
            .unwrap()
            .permit(Some(ResearchAction::CpuAffinityProfile), game(), now)
    );
    gate.lock().unwrap().observe_holdings(true, 0, holding());
    gate.lock().unwrap().observe_runtime(false, 0);
}

#[test]
fn a_long_action_study_qualifies_on_its_session_record_and_the_gates_proven_restoration() {
    let (root, mut runner, study, gate, parent) = build(&"a".repeat(64));
    assert_eq!(runner.poll(&study, 100).unwrap(), None);
    act(&gate, &parent);
    let outcome = run_to_end(&mut runner, &study);
    // The session record stands in for the native result everywhere the judgement looks.
    assert!(outcome.raw_verified && outcome.runtime_matched && outcome.context_consistent);
    assert_eq!(outcome.stage_evidence, Some(true));
    assert_eq!(
        outcome.conformance,
        vec![
            crate::autotune::research::native_context::ProfileConformance::NotSpecified;
            outcome.completed as usize
        ]
    );
    let run = judge(&study, &outcome);
    assert_eq!((run.result, run.reason), (StudyResult::Qualified, ""));

    // Independent verification replays the sealed record against the intent's own authorized unit.
    let dir =
        std::fs::File::open(root.path().join("studies").join(&study.study_sha256[..16])).unwrap();
    let (raw, _, runtime) = crate::autotune::research::child_verify::verify_child(
        &dir,
        &study.study_sha256[..16],
        crate::syscall::geteuid(),
        &parent,
        &|_| std::path::PathBuf::from("/nonexistent"),
    )
    .unwrap();
    assert!(raw && runtime, "the sealed session verifies offline");
}

#[test]
fn a_session_that_played_another_unit_cannot_qualify_the_study() {
    let (_root, mut runner, study, gate, parent) = build(&"b".repeat(64));
    assert_eq!(runner.poll(&study, 100).unwrap(), None);
    act(&gate, &parent);
    let outcome = run_to_end(&mut runner, &study);
    assert_ne!(
        judge(&study, &outcome).result,
        StudyResult::Qualified,
        "a record for another unit fails the attempt"
    );
}

/// Sends nothing anywhere: counts the phases the route entered.
struct CountingInput(std::sync::Arc<std::sync::atomic::AtomicU32>);

impl crate::autotune::research::route_driver::RouteInput for CountingInput {
    fn available(&self) -> bool {
        true
    }
    fn begin_phase(&mut self, _: &str) -> Result<(), &'static str> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
    fn stop(&mut self) {}
}

#[test]
fn the_real_route_driver_plays_the_unit_through_the_host_and_the_study_qualifies() {
    use crate::autotune::research::route_driver::{RouteDriver, RouteStep};
    let phases = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    // Time moves 400 s per look at the clock: the 1200 s route spans a few polls.
    let mut now = 50_000u64;
    let route = [("loading", 120), ("travel", 600), ("combat", 480)]
        .into_iter()
        .map(|(phase, seconds)| RouteStep {
            phase: phase.into(),
            seconds,
        })
        .collect();
    let driver = RouteDriver::new(
        CountingInput(phases.clone()),
        route,
        Box::new(move || {
            now += 400;
            now
        }),
    );
    let (root, mut runner, study, gate, parent) = build_with(driver);
    assert_eq!(runner.poll(&study, 100).unwrap(), None);
    act(&gate, &parent);
    let outcome = run_to_end(&mut runner, &study);
    assert!(outcome.raw_verified && outcome.runtime_matched);
    assert_eq!(outcome.stage_evidence, Some(true));
    let run = judge(&study, &outcome);
    assert_eq!((run.result, run.reason), (StudyResult::Qualified, ""));
    // Every attempt played all three phases of the route.
    assert_eq!(
        phases.load(std::sync::atomic::Ordering::SeqCst),
        3 * outcome.completed
    );
    // The sealed records verify offline against the intent's authorized unit.
    let dir =
        std::fs::File::open(root.path().join("studies").join(&study.study_sha256[..16])).unwrap();
    let (raw, _, runtime) = crate::autotune::research::child_verify::verify_child(
        &dir,
        &study.study_sha256[..16],
        crate::syscall::geteuid(),
        &parent,
        &|_| std::path::PathBuf::from("/nonexistent"),
    )
    .unwrap();
    assert!(raw && runtime);
}
