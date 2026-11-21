use std::sync::{Arc, Mutex};

use super::*;

#[derive(Clone, Default)]
struct Wire(Arc<Mutex<Vec<u8>>>);

impl EventSink for Wire {
    fn write(&mut self, events: &[u8]) -> Result<(), &'static str> {
        self.0.lock().unwrap().extend_from_slice(events);
        Ok(())
    }
}

struct Failing;
impl EventSink for Failing {
    fn write(&mut self, _: &[u8]) -> Result<(), &'static str> {
        Err("SinkFailed")
    }
}

/// Decode what was written: (key code, down) for each key event, checking every one is followed by a
/// synchronization report.
fn decoded(bytes: &[u8]) -> Vec<(u16, bool)> {
    assert_eq!(bytes.len() % (2 * EVENT_BYTES), 0);
    bytes
        .chunks(2 * EVENT_BYTES)
        .map(|pair| {
            let field = |at: usize| {
                (
                    u16::from_le_bytes([pair[at + 16], pair[at + 17]]),
                    u16::from_le_bytes([pair[at + 18], pair[at + 19]]),
                    i32::from_le_bytes(pair[at + 20..at + 24].try_into().unwrap()),
                )
            };
            assert_eq!(field(0).0, EV_KEY);
            assert_eq!(field(EVENT_BYTES), (EV_SYN, SYN_REPORT, 0));
            (field(0).1, field(0).2 == 1)
        })
        .collect()
}

fn script() -> Vec<PhaseScript> {
    vec![
        PhaseScript {
            phase: "travel".into(),
            steps: vec![
                KeyStep {
                    at_ms: 0,
                    key: Key::W,
                    down: true,
                },
                KeyStep {
                    at_ms: 2_000,
                    key: Key::LeftShift,
                    down: true,
                },
                KeyStep {
                    at_ms: 5_000,
                    key: Key::LeftShift,
                    down: false,
                },
            ],
        },
        PhaseScript {
            phase: "combat".into(),
            steps: vec![KeyStep {
                at_ms: 1_000,
                key: Key::F,
                down: true,
            }],
        },
    ]
}

#[test]
fn a_key_event_is_the_exact_kernel_record_followed_by_a_sync_report() {
    let bytes = encode_key(Key::W, true);
    assert_eq!(bytes.len(), 48);
    // timeval zero, type EV_KEY(1), code KEY_W(17), value 1; then EV_SYN, SYN_REPORT, 0.
    assert_eq!(&bytes[..16], &[0u8; 16]);
    assert_eq!(&bytes[16..24], &[1, 0, 17, 0, 1, 0, 0, 0]);
    assert_eq!(&bytes[24..40], &[0u8; 16]);
    assert_eq!(&bytes[40..48], &[0, 0, 0, 0, 0, 0, 0, 0]);
    let up = encode_key(Key::Escape, false);
    assert_eq!(&up[16..24], &[1, 0, 1, 0, 0, 0, 0, 0]);
}

#[test]
fn steps_are_sent_when_their_time_comes_and_never_twice() {
    let wire = Wire::default();
    let mut input = ScriptedInput::new(wire.clone(), script());
    input.begin_phase("travel").unwrap();
    assert_eq!(decoded(&wire.0.lock().unwrap()), [(17, true)]);
    input.tick(1_000).unwrap();
    assert_eq!(
        decoded(&wire.0.lock().unwrap()).len(),
        1,
        "nothing is due yet"
    );
    input.tick(2_500).unwrap();
    input.tick(2_600).unwrap();
    assert_eq!(decoded(&wire.0.lock().unwrap()), [(17, true), (42, true)]);
    input.tick(9_000).unwrap();
    assert_eq!(
        decoded(&wire.0.lock().unwrap()),
        [(17, true), (42, true), (42, false)]
    );
}

#[test]
fn no_key_is_ever_left_held_across_a_phase_a_stop_or_a_failing_sink() {
    let wire = Wire::default();
    let mut input = ScriptedInput::new(wire.clone(), script());
    input.begin_phase("travel").unwrap();
    input.tick(2_500).unwrap();
    // W and Shift are down; entering the next phase releases both before anything new is sent.
    input.begin_phase("combat").unwrap();
    let after = decoded(&wire.0.lock().unwrap());
    assert!(
        after.ends_with(&[(17, false), (42, false)])
            || after.ends_with(&[(42, false), (17, false)])
    );
    input.tick(1_000).unwrap();
    input.stop();
    let all = decoded(&wire.0.lock().unwrap());
    assert_eq!(
        all.last(),
        Some(&(33, false)),
        "stopping releases the held key"
    );
    let downs = all.iter().filter(|(_, down)| *down).count();
    let ups = all.iter().filter(|(_, down)| !*down).count();
    assert_eq!(downs, ups, "every press has its release");

    // A sink that fails is an error, and the input gives up cleanly.
    let mut failing = ScriptedInput::new(Failing, script());
    assert_eq!(failing.begin_phase("travel"), Err("SinkFailed"));
}

#[test]
fn a_phase_without_a_script_and_an_empty_input_are_refused() {
    let mut input = ScriptedInput::new(Wire::default(), script());
    assert!(input.available());
    assert_eq!(
        input.begin_phase("swimming"),
        Err("BlockedRoutePhaseHasNoScript")
    );
    assert!(!ScriptedInput::new(Wire::default(), Vec::new()).available());
}

#[test]
fn every_key_has_its_own_linux_code() {
    let mut codes: Vec<u16> = Key::ALL.iter().map(|key| key.code()).collect();
    assert_eq!(codes, [17, 30, 31, 32, 18, 33, 15, 57, 28, 1, 42]);
    codes.sort_unstable();
    codes.dedup();
    assert_eq!(codes.len(), Key::ALL.len());
}

#[test]
fn the_route_driver_feeds_the_scripted_input_its_time_and_everything_is_released_at_the_end() {
    use std::sync::atomic::{AtomicU64, Ordering};

    use crate::autotune::research::{
        long_receipt::{LongProgress, LongSessionDriver, SessionProcess},
        long_session::LongWorkUnit,
        route_driver::{RouteDriver, RouteStep},
    };
    let wire = Wire::default();
    let clock = Arc::new(AtomicU64::new(1_000));
    let shared = clock.clone();
    let route = [("travel", 600), ("combat", 600)]
        .into_iter()
        .map(|(phase, seconds)| RouteStep {
            phase: phase.into(),
            seconds,
        })
        .collect();
    let mut driver = RouteDriver::new(
        ScriptedInput::new(wire.clone(), script()),
        route,
        Box::new(move || shared.load(Ordering::SeqCst)),
    );
    let unit = LongWorkUnit {
        id: "u".into(),
        duration_seconds: 1200,
        definition_sha256: "a".repeat(64),
    };
    driver.start(&unit).unwrap();
    driver.observe_process(Some(SessionProcess {
        pid: stutter_core::ids::Pid::new(7),
        start_ticks: 1,
        uid: 1000,
    }));
    driver.poll(); // sighted: the travel phase begins and W goes down
    assert_eq!(decoded(&wire.0.lock().unwrap()), [(17, true)]);
    clock.store(1_000 + 3, Ordering::SeqCst); // three seconds in: Shift is due (2 s)
    driver.poll();
    assert_eq!(decoded(&wire.0.lock().unwrap()).last(), Some(&(42, true)));
    clock.store(1_000 + 1_300, Ordering::SeqCst); // past the whole route
    assert!(matches!(driver.poll(), LongProgress::Completed(_)));
    let all = decoded(&wire.0.lock().unwrap());
    let downs = all.iter().filter(|(_, down)| *down).count();
    let ups = all.iter().filter(|(_, down)| !*down).count();
    assert_eq!(downs, ups, "the route ends with no key held: {all:?}");
}
