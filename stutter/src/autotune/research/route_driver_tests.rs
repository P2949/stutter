use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

use super::*;
use crate::autotune::research::long_receipt::verify;

#[derive(Default)]
struct Sent {
    phases: Vec<String>,
    stops: u32,
}

struct Recording {
    sent: Arc<Mutex<Sent>>,
    available: bool,
    refuse_phase: Option<&'static str>,
}

impl RouteInput for Recording {
    fn available(&self) -> bool {
        self.available
    }
    fn begin_phase(&mut self, phase: &str) -> Result<(), &'static str> {
        if self.refuse_phase == Some(phase) {
            return Err("InputRefused");
        }
        self.sent.lock().unwrap().phases.push(phase.to_owned());
        Ok(())
    }
    fn stop(&mut self) {
        self.sent.lock().unwrap().stops += 1;
    }
}

fn unit() -> LongWorkUnit {
    LongWorkUnit {
        id: "save-a-route-1".into(),
        duration_seconds: 1200,
        definition_sha256: "a".repeat(64),
    }
}

fn route() -> Vec<RouteStep> {
    [("loading", 120), ("travel", 600), ("combat", 480)]
        .into_iter()
        .map(|(phase, seconds)| RouteStep {
            phase: phase.into(),
            seconds,
        })
        .collect()
}

fn process() -> SessionProcess {
    SessionProcess {
        pid: stutter_core::ids::Pid::new(4242),
        start_ticks: 9,
        uid: 1000,
    }
}

fn driver(available: bool) -> (RouteDriver<Recording>, Arc<Mutex<Sent>>, Arc<AtomicU64>) {
    let sent = Arc::new(Mutex::new(Sent::default()));
    let clock = Arc::new(AtomicU64::new(10_000));
    let shared = clock.clone();
    (
        RouteDriver::new(
            Recording {
                sent: sent.clone(),
                available,
                refuse_phase: None,
            },
            route(),
            Box::new(move || shared.load(Ordering::SeqCst)),
        ),
        sent,
        clock,
    )
}

#[test]
fn the_route_is_played_phase_by_phase_on_the_clock_and_the_record_is_what_happened() {
    let (mut driver, sent, clock) = driver(true);
    driver.start(&unit()).unwrap();
    // Nothing starts until the host has attested the game process.
    assert_eq!(driver.poll(), LongProgress::Running);
    assert!(sent.lock().unwrap().phases.is_empty());
    driver.observe_process(Some(process()));
    assert_eq!(driver.poll(), LongProgress::Running);
    assert_eq!(sent.lock().unwrap().phases, ["loading"]);
    // Mid-route: the travel phase has begun, nothing is complete.
    clock.store(10_000 + 130, Ordering::SeqCst);
    assert_eq!(driver.poll(), LongProgress::Running);
    assert_eq!(sent.lock().unwrap().phases, ["loading", "travel"]);
    // A late poll after the whole route closes the remaining phases at their true boundaries.
    clock.store(10_000 + 1_500, Ordering::SeqCst);
    let LongProgress::Completed(record) = driver.poll() else {
        panic!("the route is over");
    };
    assert_eq!(record.started_unix, 10_000);
    assert_eq!(record.ended_unix, 11_200);
    assert_eq!(
        record
            .phases
            .iter()
            .map(|p| (p.name.as_str(), p.started_unix, p.ended_unix))
            .collect::<Vec<_>>(),
        [
            ("loading", 10_000, 10_120),
            ("travel", 10_120, 10_720),
            ("combat", 10_720, 11_200)
        ]
    );
    assert_eq!(record.process, process());
    assert!(
        !record.settled,
        "settlement is the host's to attest, never the driver's"
    );
    assert_eq!(
        sent.lock().unwrap().stops,
        1,
        "the inputs stop when the route ends"
    );
    // Once the host attests settlement, the record is exactly what the verifier accepts.
    let mut attested = *record;
    attested.settled = true;
    assert_eq!(verify(&attested, &unit(), 1800), Ok(()));
}

#[test]
fn a_driver_that_cannot_send_input_or_a_route_of_another_length_never_starts() {
    let (mut cannot, _, _) = driver(false);
    assert_eq!(cannot.start(&unit()), Err("BlockedNoLongSessionDriver"));
    let mut shorter = unit();
    shorter.duration_seconds = 1500;
    let (mut mismatch, _, _) = driver(true);
    assert_eq!(
        mismatch.start(&shorter),
        Err("BlockedRouteDoesNotMatchTheUnit")
    );
    // The build's own input is the unavailable one.
    let mut shipped = RouteDriver::new(UnavailableInput, route(), Box::new(|| 0));
    assert_eq!(shipped.start(&unit()), Err("BlockedNoLongSessionDriver"));
}

#[test]
fn losing_or_replacing_the_game_process_ends_the_session_and_stops_the_inputs() {
    let (mut driver, sent, clock) = driver(true);
    driver.start(&unit()).unwrap();
    driver.observe_process(Some(process()));
    driver.poll();
    // The game process is gone.
    driver.observe_process(None);
    assert_eq!(
        driver.poll(),
        LongProgress::Failed("LongSessionProcessLost")
    );
    assert!(sent.lock().unwrap().stops >= 1);

    // A different process (a restart) is not the same session.
    let (mut other, _, _) = self::driver(true);
    other.start(&unit()).unwrap();
    other.observe_process(Some(process()));
    other.poll();
    other.observe_process(Some(SessionProcess {
        start_ticks: 10,
        ..process()
    }));
    assert_eq!(other.poll(), LongProgress::Failed("LongSessionProcessLost"));
    let _ = clock;
}

#[test]
fn an_input_the_backend_refuses_and_an_abort_both_stop_sending_inputs() {
    let sent = Arc::new(Mutex::new(Sent::default()));
    let clock = Arc::new(AtomicU64::new(0));
    let shared = clock.clone();
    let mut refusing = RouteDriver::new(
        Recording {
            sent: sent.clone(),
            available: true,
            refuse_phase: Some("travel"),
        },
        route(),
        Box::new(move || shared.load(Ordering::SeqCst)),
    );
    refusing.start(&unit()).unwrap();
    refusing.observe_process(Some(process()));
    refusing.poll();
    clock.store(200, Ordering::SeqCst);
    assert_eq!(refusing.poll(), LongProgress::Failed("InputRefused"));
    assert!(sent.lock().unwrap().stops >= 1);

    let (mut aborted, sent, _) = driver(true);
    aborted.start(&unit()).unwrap();
    aborted.observe_process(Some(process()));
    aborted.poll();
    aborted.abort().unwrap();
    assert!(sent.lock().unwrap().stops >= 1);
    assert_eq!(
        aborted.poll(),
        LongProgress::Failed("LongSessionNotStarted")
    );
}
