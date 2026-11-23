//! A CP-LONG action study run through the real `DriverStudyAttempts` over the long host: the attempt is a
//! sealed session record instead of a native benchmark result, the study qualifies on the gate's proven
//! restoration, and independent verification replays the session record against the intent's unit.

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
        definition_sha256: crate::autotune::research::route_driver::definition_sha256_of(
            &[("loading", 120), ("travel", 600), ("combat", 480)].map(|(phase, seconds)| {
                crate::autotune::research::route_driver::RouteStep {
                    phase: phase.into(),
                    seconds,
                }
            }),
            "",
        ),
    }
}

thread_local! {
    /// The unix second the controller is shown applying the action at: inside every session's window (a route
    /// that keeps its own clock advances it).
    static ACTED_AT: std::cell::Cell<u64> = const { std::cell::Cell::new(5_100) };
}

/// The game process of the Nth attempt: every launch is a new process.
fn attempt_process(slot: u32) -> ChildTarget {
    ChildTarget {
        pid: stutter_core::ids::Pid::new(5_000 + slot),
        start_ticks: 100 + u64::from(slot),
        uid: crate::syscall::geteuid(),
    }
}

thread_local! {
    /// What the test needs of the one study it runs: the process the host last attested and the worker tokens the
    /// gate's probe reads.
    static RIG: std::cell::RefCell<Option<(Sighted, std::sync::Arc<std::sync::atomic::AtomicU32>)>> =
        const { std::cell::RefCell::new(None) };
}

type Sighted = std::sync::Arc<std::sync::Mutex<Option<SessionProcess>>>;

/// Records the process the host attested to the route, and delegates everything else.
struct Observing<D>(D, Sighted);

impl<D: LongSessionDriver> LongSessionDriver for Observing<D> {
    fn observe_process(&mut self, process: Option<SessionProcess>) {
        *self.1.lock().unwrap() = process;
        self.0.observe_process(process);
    }
    fn start(&mut self, unit: &LongWorkUnit) -> Result<(), &'static str> {
        self.0.start(unit)
    }
    fn poll(&mut self) -> LongProgress {
        self.0.poll()
    }
    fn abort(&mut self) -> anyhow::Result<()> {
        self.0.abort()
    }
}

/// Plays the route to completion on the second poll of every session, as the unit says it must, as whichever
/// game process the host sighted for that attempt.
struct Route {
    definition: String,
    sighted: Option<SessionProcess>,
    polls: u32,
}

impl LongSessionDriver for Route {
    fn observe_process(&mut self, process: Option<SessionProcess>) {
        self.sighted = process;
    }
    fn start(&mut self, _: &LongWorkUnit) -> Result<(), &'static str> {
        self.polls = 0;
        Ok(())
    }
    fn poll(&mut self) -> LongProgress {
        self.polls += 1;
        if self.polls < 2 {
            return LongProgress::Running;
        }
        // The record names the process the host sighted on this very attempt.
        let mut record = session(&self.definition);
        if let Some(process) = self.sighted {
            record.process = process;
        }
        LongProgress::Completed(Box::new(record))
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
        max_poll_gap_seconds: Some(1),
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
        sighted: None,
        polls: 0,
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
    // The sessions sit at unix 5000 and later, as twenty-minute routes do.
    parent.issued_unix_seconds = 4_000;
    parent.deadline_unix_seconds = 4_000 + 40_000;
    parent.long_attempt_seconds = Some(1800);
    parent.long_work_unit = Some(unit());
    parent.validate().unwrap();
    let study = plan(&parent)
        .unwrap()
        .into_iter()
        .find(|s| matches!(s.stage, Stage::Action(_)))
        .unwrap();
    let tokens = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    ACTED_AT.set(5_100);
    let held = tokens.clone();
    let gate: Shared = std::sync::Arc::new(std::sync::Mutex::new(
        ActionGate::unarmed(Box::new(move || {
            Some(held.load(std::sync::atomic::Ordering::SeqCst))
        }))
        .with_clock(Box::new(|| ACTED_AT.get() + 1))
        .with_state_probe(Box::new(|pid| {
            Some(KernelState {
                cpus_allowed: "0-15".into(),
                cgroup: "0::/app.slice".into(),
                tasks: [(pid, "0-15".to_owned())].into(),
                identity: Some((u64::from(pid) - 4_900, crate::syscall::geteuid())),
            })
        })),
    ));
    // Every attempt is its own game process, and the host follows the study's own schedule.
    let sighted = std::sync::Arc::new(std::sync::Mutex::new(None));
    let driver = Observing(driver, sighted.clone());
    let mut inner = Host::finishing();
    inner.processes = (0..64).map(attempt_process).collect();
    inner.schedule_out = Some(Default::default());
    let host = LongAttemptHost::new(inner, driver);
    RIG.with(|rig| *rig.borrow_mut() = Some((sighted, tokens)));
    let base = parent.issued_unix_seconds + 1;
    let mut calls = 0u64;
    let attempts = DriverStudyAttempts::new(
        File::open(root.path()).unwrap(),
        parent.clone(),
        Box::new(host),
        // A real route is polled every two seconds for twenty minutes, so the controller's own clock is read
        // far more often than once per second of the study; it advances one second per 64 reads here.
        Box::new(move || {
            calls += 1;
            base + calls / 64
        }),
        Box::new(|| Ok(())),
        Box::new(|_| std::path::PathBuf::from("/nonexistent")),
        Box::new(|study| Ok(inputs_for(study))),
    )
    .with_action_gate(gate.clone());
    (root, attempts, study, gate, parent)
}

/// Runs the study to its end while the normal controller applies the action once in every attempt the host sights
/// (the gate refuses the ones the schedule leaves untreated) and restores it, as the gate observes.
fn run_acting(
    runner: &mut DriverStudyAttempts,
    study: &ChildStudy,
    gate: &Shared,
) -> AttemptsOutcome {
    let (sighted, tokens) = RIG.with(|rig| rig.borrow().clone().expect("a built study"));
    let mut acted = std::collections::BTreeSet::new();
    for _ in 0..20_000 {
        if let Some(outcome) = runner.poll(study, 100).unwrap() {
            return outcome;
        }
        let Some(process) = *sighted.lock().unwrap() else {
            continue;
        };
        let pid = process.pid;
        if acted.contains(&pid) {
            continue;
        }
        let target = ChildTarget {
            pid,
            start_ticks: process.start_ticks,
            uid: process.uid,
        };
        let mut gate = gate.lock().unwrap();
        if gate.permit(
            Some(ResearchAction::CpuAffinityProfile),
            target,
            ACTED_AT.get(),
        ) {
            tokens.store(1, std::sync::atomic::Ordering::SeqCst);
            gate.observe_holdings(
                true,
                0,
                vec![Holding {
                    experiment_id: stutter_core::ids::ExperimentId::new("exp-1"),
                    action: Some(ResearchAction::CpuAffinityProfile),
                    pid: Some(pid),
                }],
            );
            tokens.store(0, std::sync::atomic::Ordering::SeqCst);
            gate.observe_runtime(false, 0);
            acted.insert(pid);
        }
    }
    panic!("the study never ended")
}

#[test]
fn a_long_action_study_qualifies_on_its_session_record_and_the_gates_proven_restoration() {
    let (root, mut runner, study, gate, parent) = build(&unit().definition_sha256);
    let outcome = run_acting(&mut runner, &study, &gate);
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
    assert_eq!(
        (run.result, run.reason),
        (StudyResult::Unqualified, "ActionMetricUnavailable")
    );

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
    let (_root, mut runner, study, gate, _parent) = build(&"b".repeat(64));
    let outcome = run_acting(&mut runner, &study, &gate);
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
    // Time moves 2 s per look at the clock, the most the driver allows between polls.
    let mut now = 5_000u64;
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
            now += 2;
            ACTED_AT.set(now);
            now
        }),
    );
    let (root, mut runner, study, gate, parent) = build_with(driver);
    let outcome = run_acting(&mut runner, &study, &gate);
    assert!(outcome.raw_verified && outcome.runtime_matched);
    assert_eq!(outcome.stage_evidence, Some(true));
    let run = judge(&study, &outcome);
    assert_eq!(
        (run.result, run.reason),
        (StudyResult::Unqualified, "ActionMetricUnavailable")
    );
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

/// A qualified action's `stage_evidence` is rebuilt for replay from the sealed receipts and the schedule, so the
/// shared judgement accepts the live qualification; forged, missing, partial or misattributed receipts do not.
#[test]
fn the_replayed_action_evidence_needs_a_valid_receipt_on_every_treated_attempt_of_its_own_process()
{
    use std::{collections::BTreeMap, os::unix::fs::PermissionsExt};

    use crate::autotune::research::{
        child_judgement::action_replay, report::AttemptEvidenceV1, treatment::treatments_of,
    };
    let (root, mut runner, study, gate, _parent) = build(&unit().definition_sha256);
    let outcome = run_acting(&mut runner, &study, &gate);
    assert_eq!(judge(&study, &outcome).result, StudyResult::Unqualified);
    let path = root.path().join("studies").join(&study.study_sha256[..16]);
    let dir = std::fs::File::open(&path).unwrap();
    let uid = crate::syscall::geteuid();
    let intent: ResearchIntentV1 =
        serde_json::from_slice(&std::fs::read(path.join("intent.json")).unwrap()).unwrap();
    let schedule = treatments_of(&intent).unwrap().unwrap();
    // One distinct game process per attempt, as a real campaign has.
    let evidence: BTreeMap<u32, AttemptEvidenceV1> = (0..schedule.len() as u32)
        .map(|slot| {
            let mut record = session(&unit().definition_sha256);
            record.process = SessionProcess {
                pid: stutter_core::ids::Pid::new(5000 + slot),
                start_ticks: 100 + u64::from(slot),
                uid,
            };
            let item = AttemptEvidenceV1 {
                schema_version: 1,
                slot_index: slot,
                average_fps: 1.0,
                native_completion: None,
                long_session: Some(Box::new(record)),
                thermal: None,
            };
            (slot, item)
        })
        .collect();
    let state = |pid: u32, ticks: u64| {
        serde_json::json!({
            "cgroup": "0::/app.slice", "cpus_allowed": "0-15",
            "tasks": { pid.to_string(): "0-15" }, "identity": [ticks, uid],
        })
    };
    let receipt = |slot: u32| {
        let (pid, ticks) = (5000 + slot, 100 + u64::from(slot));
        serde_json::json!({
            "transaction": format!("exp-{slot}"), "action": "CpuAffinityProfile",
            "pid": pid, "start_ticks": ticks, "uid": uid,
            "permitted_unix": 5_100, "applied_unix": 5_101, "settled_unix": 5_102, "peak_tokens": 1,
            "original": state(pid, ticks), "restored": state(pid, ticks),
        })
    };
    let treated: Vec<u32> = schedule
        .iter()
        .enumerate()
        .filter(|(_, t)| **t == crate::autotune::research::Treatment::On)
        .map(|(slot, _)| slot as u32)
        .collect();
    let untreated = (0..schedule.len() as u32)
        .find(|slot| !treated.contains(slot))
        .unwrap();
    let write = |items: Vec<serde_json::Value>| {
        let file = path.join("action-receipts.json");
        let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600));
        std::fs::write(&file, serde_json::to_vec(&items).unwrap()).unwrap();
    };
    let replay = || {
        action_replay(
            &dir,
            uid,
            uid,
            ResearchAction::CpuAffinityProfile,
            &intent,
            &evidence,
        )
    };

    write(treated.iter().map(|slot| receipt(*slot)).collect());
    assert!(replay(), "every treated attempt has its own valid receipt");
    // The worker-transaction timeline: missing, out of order, no worker token, or outside the attempt each fail.
    let with = |edit: &dyn Fn(&mut serde_json::Value)| {
        let mut items: Vec<_> = treated.iter().map(|slot| receipt(*slot)).collect();
        edit(&mut items[0]);
        write(items);
        replay()
    };
    assert!(
        !with(&|r| {
            r.as_object_mut().unwrap().remove("settled_unix");
        }),
        "no timeline"
    );
    assert!(
        !with(&|r| r["applied_unix"] = 5_200.into()),
        "applied after it settled"
    );
    assert!(
        !with(&|r| r["peak_tokens"] = 0.into()),
        "the root worker never held a token"
    );
    assert!(
        !with(&|r| r["permitted_unix"] = 4_000.into()),
        "permitted before the attempt began"
    );
    assert!(
        !with(&|r| r["settled_unix"] = 7_000.into()),
        "settled after the attempt ended"
    );
    write(treated.iter().map(|slot| receipt(*slot)).collect());
    // A worker thread that exited while the action ran: valid only when the receipt names it, the post-restore
    // listing no longer has it, and it is not the leader.
    let with_exit = |named: Option<serde_json::Value>, still_listed: bool| {
        treated
            .iter()
            .enumerate()
            .map(|(index, slot)| {
                let mut item = receipt(*slot);
                if index == 0 {
                    let tid = 9_000 + slot;
                    item["original"]["tasks"][tid.to_string()] = "0-15".into();
                    if still_listed {
                        item["restored"]["tasks"][tid.to_string()] = "0-15".into();
                    }
                    if let Some(named) = &named {
                        item["exited"] = named.clone();
                    }
                }
                item
            })
            .collect::<Vec<_>>()
    };
    let first_tid = 9_000 + treated[0];
    write(with_exit(Some(serde_json::json!([first_tid])), false));
    assert!(replay(), "a named, witnessed exit is not a difference");
    write(with_exit(None, false));
    assert!(!replay(), "an unnamed absence is unknown, never an exit");
    write(with_exit(Some(serde_json::json!([first_tid])), true));
    assert!(!replay(), "a thread still listed has not exited");
    write(with_exit(
        Some(serde_json::json!([5000 + treated[0]])),
        false,
    ));
    assert!(!replay(), "the leader cannot be the thread that exited");
    write(treated.iter().map(|slot| receipt(*slot)).collect());
    // One treated attempt without a receipt is a partly applied action.
    write(treated[1..].iter().map(|slot| receipt(*slot)).collect());
    assert!(!replay());
    // A receipt on an attempt the schedule left untreated is not an applied action.
    let mut extra: Vec<_> = treated.iter().map(|slot| receipt(*slot)).collect();
    extra.push(receipt(untreated));
    write(extra);
    assert!(!replay());
    // A receipt whose restored placement differs is not restoration.
    let mut broken = treated
        .iter()
        .map(|slot| receipt(*slot))
        .collect::<Vec<_>>();
    broken[0]["restored"]["cpus_allowed"] = "0-3".into();
    write(broken);
    assert!(!replay());
    // No receipts file at all.
    std::fs::remove_file(path.join("action-receipts.json")).unwrap();
    assert!(!replay());
}

/// An action study during which the controller never applies the action is a justified abstention, not a
/// qualified action and not a failure: its own record replays, and the gates need the control beside it.
#[test]
fn a_study_in_which_the_action_was_never_applied_is_a_replayable_justified_abstention() {
    use crate::autotune::research::{
        child_judgement::verify_qualification, child_ledger::Disposition,
    };
    let (root, mut runner, study, _gate, parent) = build(&unit().definition_sha256);
    assert_eq!(runner.poll(&study, 100).unwrap(), None);
    // No `act`: the controller saw nothing to do for the whole study.
    let outcome = run_to_end(&mut runner, &study);
    assert_eq!(outcome.stage_evidence, Some(false));
    assert!(outcome.action_not_indicated);
    let run = judge(&study, &outcome);
    assert_eq!(
        (run.result, run.reason),
        (StudyResult::Unqualified, "ActionNotIndicated")
    );
    let path = root.path().join("studies").join(&study.study_sha256[..16]);
    let dir = std::fs::File::open(&path).unwrap();
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path.join("report.json")).unwrap()).unwrap();
    let disposition = Disposition {
        study_sha256: study.study_sha256.clone(),
        profile: study.profile,
        stage: study.stage,
        result: Some(StudyResult::Unqualified),
        reason: "ActionNotIndicated".into(),
    };
    let uid = crate::syscall::geteuid();
    let check = |disposition: &Disposition| {
        verify_qualification(&dir, uid, &parent, disposition, &report, true, true)
    };
    check(&disposition).unwrap();
    // Receipts beside a claimed abstention mean an action was applied: the claim is contradicted.
    std::fs::write(path.join("action-receipts.json"), b"[]").unwrap();
    assert!(check(&disposition).is_err());
}

/// A long session's sealed placeholder FPS is never analysed as a measurement.
#[test]
fn a_long_action_study_reports_no_numeric_result_from_its_placeholder_fps() {
    let (root, mut runner, study, gate, _parent) = build(&unit().definition_sha256);
    run_acting(&mut runner, &study, &gate);
    let path = root.path().join("studies").join(&study.study_sha256[..16]);
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path.join("report.json")).unwrap()).unwrap();
    let result = report
        .pointer("/arms/0")
        .or_else(|| report.get("treatment"))
        .expect("the report's one result");
    assert_eq!(result["status"], "ActionMetricUnavailable", "{report}");
    assert!(result["numeric_verdict"].is_null() && result["mean_effect_percent"].is_null());
}

#[path = "driver_attempts_long_qualified_tests.rs"]
mod qualified;
