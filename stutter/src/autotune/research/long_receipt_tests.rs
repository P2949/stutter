use super::*;

const WINDOW: u32 = 30 * 60;

fn unit() -> LongWorkUnit {
    LongWorkUnit {
        id: "save-12-route-a".into(),
        duration_seconds: 25 * 60,
        definition_sha256: "ab".repeat(32),
    }
}

/// A complete session of the unit: 26 minutes in three phases that cover it.
fn record() -> LongSessionRecord {
    let start = 10_000;
    LongSessionRecord {
        unit_id: unit().id,
        definition_sha256: unit().definition_sha256,
        started_unix: start,
        ended_unix: start + 26 * 60,
        process: SessionProcess {
            pid: stutter_core::ids::Pid::new(4242),
            start_ticks: 99,
            uid: 1000,
        },
        phases: vec![
            Phase {
                name: "load".into(),
                started_unix: start,
                ended_unix: start + 120,
            },
            Phase {
                name: "travel".into(),
                started_unix: start + 120,
                ended_unix: start + 20 * 60,
            },
            Phase {
                name: "combat".into(),
                started_unix: start + 20 * 60,
                ended_unix: start + 26 * 60,
            },
        ],
        max_poll_gap_seconds: Some(1),
        settled: true,
    }
}

#[test]
fn a_complete_settled_session_of_the_authorized_unit_inside_its_window_verifies() {
    assert_eq!(verify(&record(), &unit(), WINDOW), Ok(()));
}

#[test]
fn every_way_a_record_can_fail_to_stand_for_the_session_is_its_own_refusal() {
    let refused = |edit: &dyn Fn(&mut LongSessionRecord), expected: &'static str| {
        let mut bad = record();
        edit(&mut bad);
        assert_eq!(verify(&bad, &unit(), WINDOW), Err(expected));
    };
    // Another unit, or one whose definition (save, route, conditions) differs.
    refused(
        &|r| r.unit_id = "other".into(),
        "LongSessionNotTheAuthorizedUnit",
    );
    refused(
        &|r| r.definition_sha256 = "cd".repeat(32),
        "LongSessionNotTheAuthorizedUnit",
    );
    // The benchmark's few minutes are not the route; time cannot run backwards.
    refused(
        &|r| r.ended_unix = r.started_unix + 300,
        "LongSessionShorterThanTheUnit",
    );
    refused(
        &|r| r.ended_unix = r.started_unix - 1,
        "LongSessionTimeReversed",
    );
    // Past the authorized window.
    refused(
        &|r| {
            r.ended_unix = r.started_unix + u64::from(WINDOW) + 1;
            r.phases.last_mut().unwrap().ended_unix = r.ended_unix;
        },
        "LongSessionOutlastedTheAuthorizedWindow",
    );
    refused(
        &|r| r.process.pid = stutter_core::ids::Pid::new(0),
        "LongSessionProcessNotIdentified",
    );
    refused(
        &|r| r.process.start_ticks = 0,
        "LongSessionProcessNotIdentified",
    );
    refused(&|r| r.settled = false, "LongSessionNotSettled");
    // Phases that overlap, run past the end, or leave most of the session unobserved.
    refused(
        &|r| r.phases[1].started_unix -= 10,
        "LongSessionPhasesInconsistent",
    );
    refused(
        &|r| r.phases[2].ended_unix = r.ended_unix + 1,
        "LongSessionPhasesInconsistent",
    );
    refused(&|r| r.phases.truncate(1), "LongSessionMostlyUnobserved");
    refused(&|r| r.phases.clear(), "LongSessionMostlyUnobserved");
}

#[test]
fn this_build_has_no_driver_and_says_so_instead_of_running_the_benchmark() {
    let mut driver = UnavailableDriver;
    assert_eq!(driver.start(&unit()), Err("BlockedNoLongSessionDriver"));
    assert_eq!(
        driver.poll(),
        LongProgress::Failed("BlockedNoLongSessionDriver")
    );
    assert!(driver.abort().is_ok());
}

/// A scripted driver, to show the lifecycle a real one follows and that its output is judged only by
/// the verifier, not by what the driver claims.
struct Scripted(Option<LongProgress>);

impl LongSessionDriver for Scripted {
    fn start(&mut self, _unit: &LongWorkUnit) -> Result<(), &'static str> {
        Ok(())
    }
    fn poll(&mut self) -> LongProgress {
        self.0.take().unwrap_or(LongProgress::Running)
    }
    fn abort(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
}

#[test]
fn a_drivers_completed_session_is_accepted_only_through_the_verifier() {
    let mut honest = Scripted(Some(LongProgress::Completed(Box::new(record()))));
    honest.start(&unit()).unwrap();
    let LongProgress::Completed(done) = honest.poll() else {
        panic!("expected a completed session")
    };
    assert_eq!(verify(&done, &unit(), WINDOW), Ok(()));
    // A driver that claims completion of a session it cut short is caught by the verifier.
    let mut short = record();
    short.ended_unix = short.started_unix + 300;
    short.phases = vec![Phase {
        name: "load".into(),
        started_unix: short.started_unix,
        ended_unix: short.ended_unix,
    }];
    let mut cheat = Scripted(Some(LongProgress::Completed(Box::new(short))));
    let LongProgress::Completed(claimed) = cheat.poll() else {
        panic!("expected a completed session")
    };
    assert_eq!(
        verify(&claimed, &unit(), WINDOW),
        Err("LongSessionShorterThanTheUnit")
    );
}

#[test]
fn a_session_must_show_how_closely_its_route_was_watched() {
    let mut unproven = record();
    unproven.max_poll_gap_seconds = None;
    assert_eq!(
        verify(&unproven, &unit(), WINDOW),
        Err("LongSessionDeliveryUnproven")
    );
    let mut gappy = record();
    gappy.max_poll_gap_seconds =
        Some(crate::autotune::research::route_driver::MAX_POLL_GAP_SECONDS + 1);
    assert_eq!(verify(&gappy, &unit(), WINDOW), Err("LongSessionInputGap"));
    let mut at_bound = record();
    at_bound.max_poll_gap_seconds =
        Some(crate::autotune::research::route_driver::MAX_POLL_GAP_SECONDS);
    assert_eq!(verify(&at_bound, &unit(), WINDOW), Ok(()));
}
