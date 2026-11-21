use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use super::*;
use crate::autotune::research::long_receipt::{
    LongSessionRecord, Phase, SessionProcess, UnavailableDriver,
};

#[derive(Default)]
struct Calls {
    launched: u32,
    inner_aborts: u32,
    driver_aborts: u32,
}

struct Inner {
    calls: Arc<Mutex<Calls>>,
    poll: Option<Progress>,
    abort_fails: bool,
}

impl AttemptHost for Inner {
    fn safe_window(&mut self) -> bool {
        true
    }
    fn arm_spec(&mut self, _: &AttemptSlot) -> anyhow::Result<ArmSpec> {
        unreachable!("not armed in these tests")
    }
    fn attempt_index(&mut self) -> Option<u32> {
        Some(0)
    }
    fn launch(&mut self, _: &AttemptSlot, _: &DispatchContextV1) -> anyhow::Result<()> {
        self.calls.lock().unwrap().launched += 1;
        Ok(())
    }
    fn poll(&mut self) -> Progress {
        self.poll.take().unwrap_or(Progress::Running)
    }
    fn abort(&mut self) -> anyhow::Result<()> {
        self.calls.lock().unwrap().inner_aborts += 1;
        anyhow::ensure!(!self.abort_fails, "unsettled");
        Ok(())
    }
}

struct Scripted {
    calls: Arc<Mutex<Calls>>,
    states: VecDeque<LongProgress>,
}

impl LongSessionDriver for Scripted {
    fn start(&mut self, _: &LongWorkUnit) -> Result<(), &'static str> {
        Ok(())
    }
    fn poll(&mut self) -> LongProgress {
        self.states.pop_front().unwrap_or(LongProgress::Running)
    }
    fn abort(&mut self) -> anyhow::Result<()> {
        self.calls.lock().unwrap().driver_aborts += 1;
        Ok(())
    }
}

fn unit() -> LongWorkUnit {
    LongWorkUnit {
        id: "save-a-route-1".into(),
        duration_seconds: 1200,
        definition_sha256: "a".repeat(64),
    }
}

fn record(definition: &str) -> LongSessionRecord {
    LongSessionRecord {
        unit_id: "save-a-route-1".into(),
        definition_sha256: definition.into(),
        started_unix: 1000,
        ended_unix: 1000 + 1200,
        process: SessionProcess {
            pid: stutter_core::ids::Pid::new(4242),
            start_ticks: 9,
            uid: 1000,
        },
        phases: vec![Phase {
            name: "route".into(),
            started_unix: 1000,
            ended_unix: 2200,
        }],
        settled: true,
    }
}

fn context(with_unit: bool, long: bool) -> DispatchContextV1 {
    let mut intent = crate::autotune::research::tests::intent();
    intent.child_study_sha256 = Some("c".repeat(64));
    if long {
        intent.long_attempt_seconds = Some(1800);
        intent.max_attempt_seconds = 1800;
        intent.long_work_unit = with_unit.then(unit);
    }
    DispatchContextV1 {
        schema_version: 1,
        slot: slot(),
        intent,
        tickets: Default::default(),
        spec: ArmSpec {
            tool_dir: "/t".into(),
            study_compat_data: "/s".into(),
            arm_env: Default::default(),
            steamapps: "/a".into(),
            proton_manifest: "/m".into(),
        },
    }
}

fn slot() -> AttemptSlot {
    AttemptSlot {
        study_sha256: "e".repeat(64),
        context_sha256: "f".repeat(64),
        index: 0,
        arm: ProtonSource::Ge,
    }
}

fn host(
    states: Vec<LongProgress>,
    abort_fails: bool,
) -> (LongAttemptHost<Inner, Scripted>, Arc<Mutex<Calls>>) {
    let calls = Arc::new(Mutex::new(Calls::default()));
    (
        LongAttemptHost::new(
            Inner {
                calls: calls.clone(),
                poll: None,
                abort_fails,
            },
            Scripted {
                calls: calls.clone(),
                states: states.into(),
            },
        ),
        calls,
    )
}

#[test]
fn a_verified_session_finishes_the_attempt_with_its_record_after_both_halves_settle() {
    let (mut host, calls) = host(
        vec![
            LongProgress::Running,
            LongProgress::Completed(Box::new(record(&"a".repeat(64)))),
        ],
        false,
    );
    host.launch(&slot(), &context(true, true)).unwrap();
    assert!(matches!(host.poll(), Progress::Running));
    let Progress::Finished { evidence } = host.poll() else {
        panic!("a verified session finishes the attempt");
    };
    assert!(evidence.native_completion.is_none());
    assert_eq!(
        evidence.long_session.as_deref().unwrap().unit_id,
        "save-a-route-1"
    );
    let calls = calls.lock().unwrap();
    assert_eq!((calls.inner_aborts, calls.driver_aborts), (1, 1));
}

#[test]
fn a_record_for_another_unit_or_an_unsettled_game_never_finishes_it() {
    let (mut host, calls) = host(
        vec![LongProgress::Completed(Box::new(record(&"b".repeat(64))))],
        false,
    );
    host.launch(&slot(), &context(true, true)).unwrap();
    assert!(matches!(
        host.poll(),
        Progress::Failed { code, .. } if code == "LongSessionNotTheAuthorizedUnit"
    ));
    assert_eq!(
        calls.lock().unwrap().inner_aborts,
        1,
        "the game is stopped first"
    );

    let (mut stuck, _) = host_stuck();
    stuck.launch(&slot(), &context(true, true)).unwrap();
    assert!(matches!(
        stuck.poll(),
        Progress::Failed { code, .. } if code == "SettlementUnverified"
    ));
}

fn host_stuck() -> (LongAttemptHost<Inner, Scripted>, Arc<Mutex<Calls>>) {
    host(
        vec![LongProgress::Completed(Box::new(record(&"a".repeat(64))))],
        true,
    )
}

#[test]
fn without_a_driver_or_a_unit_a_long_attempt_is_refused_with_its_typed_reason() {
    let calls = Arc::new(Mutex::new(Calls::default()));
    let mut none = LongAttemptHost::new(
        Inner {
            calls: calls.clone(),
            poll: None,
            abort_fails: false,
        },
        UnavailableDriver,
    );
    let error = none.launch(&slot(), &context(true, true)).unwrap_err();
    assert_eq!(
        error.downcast_ref::<LaunchFailure>().unwrap().code,
        "BlockedNoLongSessionDriver"
    );
    assert_eq!(
        calls.lock().unwrap().inner_aborts,
        1,
        "the launched game is stopped"
    );

    let (mut unitless, calls) = host(vec![], false);
    let error = unitless.launch(&slot(), &context(false, true)).unwrap_err();
    assert_eq!(
        error.downcast_ref::<LaunchFailure>().unwrap().code,
        "BlockedNoComparableLongWorkUnit"
    );
    assert_eq!(calls.lock().unwrap().launched, 0, "nothing was launched");
}

#[test]
fn a_game_failure_wins_and_every_other_attempt_is_delegated_untouched() {
    let (mut failing, calls) = host(vec![], false);
    failing.inner.poll = Some(Progress::Failed {
        class: FailureClass::Health,
        code: "GpuReset".into(),
    });
    failing.launch(&slot(), &context(true, true)).unwrap();
    assert!(matches!(
        failing.poll(),
        Progress::Failed { code, .. } if code == "GpuReset"
    ));
    assert_eq!(calls.lock().unwrap().driver_aborts, 1);

    // A benchmark attempt never touches the driver.
    let (mut plain, calls) = host(vec![LongProgress::Failed("never")], false);
    plain.inner.poll = Some(Progress::Running);
    plain.launch(&slot(), &context(true, false)).unwrap();
    assert!(matches!(plain.poll(), Progress::Running));
    assert_eq!(calls.lock().unwrap().driver_aborts, 0);
}
