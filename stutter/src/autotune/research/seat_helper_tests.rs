use std::io::Cursor;

use super::*;
use crate::autotune::research::scripted_input::{
    encode_key,
    isolated_seat::{LiveSession, attest},
};

#[derive(Default)]
struct Keyboard {
    log: Vec<(Key, bool)>,
    refuse_release: bool,
    refuse_after: Option<usize>,
}

impl Injector for Keyboard {
    fn inject(&mut self, key: Key, down: bool) -> Result<(), &'static str> {
        if !down && self.refuse_release {
            return Err("refused");
        }
        if self
            .refuse_after
            .is_some_and(|limit| self.log.len() >= limit)
        {
            return Err("refused");
        }
        self.log.push((key, down));
        Ok(())
    }
}

fn seat() -> IsolatedSeat {
    let live = LiveSession {
        wayland_display: "wayland-1".into(),
        x_display: ":0".into(),
        session_id: "11".into(),
    };
    attest(
        b"WAYLAND_DISPLAY=wayland-77\0DISPLAY=:77\0XDG_SESSION_ID=42\0XDG_SEAT=seat7\0",
        &live,
    )
    .unwrap()
}

fn stream(steps: &[(Key, bool)]) -> Vec<u8> {
    steps
        .iter()
        .flat_map(|(key, down)| encode_key(*key, *down))
        .collect()
}

#[test]
fn a_route_is_injected_in_order_and_a_clean_end_leaves_nothing_down() {
    let mut helper = SeatHelper::new(Keyboard::default(), &seat());
    assert_eq!(helper.display(), "wayland-77");
    let bytes = stream(&[
        (Key::W, true),
        (Key::W, false),
        (Key::E, true),
        (Key::E, false),
    ]);
    let served = helper.serve(&mut Cursor::new(bytes), &mut Vec::new());
    assert_eq!(
        served,
        Served {
            injected: 4,
            stopped: None,
            all_released: true
        }
    );
    assert_eq!(helper.injector.log.len(), 4);
}

#[test]
fn a_connection_that_ends_while_keys_are_down_releases_every_one_of_them() {
    // The route pressed two keys and the connection ended (or was cut) before it released them.
    let mut helper = SeatHelper::new(Keyboard::default(), &seat());
    let bytes = stream(&[(Key::W, true), (Key::LeftShift, true)]);
    let served = helper.serve(&mut Cursor::new(bytes), &mut Vec::new());
    assert_eq!(
        (served.injected, served.stopped, served.all_released),
        (2, None, true)
    );
    let mut released: Vec<_> = helper
        .injector
        .log
        .iter()
        .filter(|(_, down)| !down)
        .map(|(key, _)| key.code())
        .collect();
    released.sort_unstable();
    assert_eq!(released, vec![Key::W.code(), Key::LeftShift.code()]);
    // A torn record is not a clean end, and releases too.
    let mut helper = SeatHelper::new(Keyboard::default(), &seat());
    let mut bytes = stream(&[(Key::W, true)]);
    bytes.extend_from_slice(&stream(&[(Key::A, true)])[..10]);
    let served = helper.serve(&mut Cursor::new(bytes), &mut Vec::new());
    assert_eq!(
        (served.injected, served.stopped, served.all_released),
        (1, Some("SinkStreamTorn"), true)
    );
    assert_eq!(helper.injector.log.last(), Some(&(Key::W, false)));
}

#[test]
fn an_event_a_route_could_not_send_or_a_failed_injection_stops_it_and_a_refused_release_is_reported()
 {
    // Another key, another event type: refused, and the key already down is released.
    let mut helper = SeatHelper::new(Keyboard::default(), &seat());
    let mut bytes = stream(&[(Key::W, true)]);
    let mut other = stream(&[(Key::A, true)]);
    other[18..20].copy_from_slice(&200u16.to_le_bytes());
    bytes.extend_from_slice(&other);
    let served = helper.serve(&mut Cursor::new(bytes), &mut Vec::new());
    assert_eq!(
        (served.injected, served.stopped, served.all_released),
        (1, Some("SinkEventRefused"), true)
    );
    assert_eq!(helper.injector.log.last(), Some(&(Key::W, false)));
    // The device refuses an injection mid-route: it stops, and releases what it held.
    let mut helper = SeatHelper::new(
        Keyboard {
            refuse_after: Some(1),
            ..Default::default()
        },
        &seat(),
    );
    let served = helper.serve(
        &mut Cursor::new(stream(&[(Key::W, true), (Key::E, true)])),
        &mut Vec::new(),
    );
    assert_eq!(served.stopped, Some("InjectionFailed"));
    // A release the device will not accept is owed and reported, never claimed done.
    let mut helper = SeatHelper::new(
        Keyboard {
            refuse_release: true,
            ..Default::default()
        },
        &seat(),
    );
    let served = helper.serve(&mut Cursor::new(stream(&[(Key::W, true)])), &mut Vec::new());
    assert_eq!((served.stopped, served.all_released), (None, false));
}

/// The helper acknowledges each key transition it injected and none it did not: the sender can tell a delivered write from
/// one that was only read.
#[test]
fn the_helper_acknowledges_exactly_the_transitions_it_injected() {
    let mut helper = SeatHelper::new(Keyboard::default(), &seat());
    let mut acks = Vec::new();
    let served = helper.serve(
        &mut Cursor::new(stream(&[(Key::W, true), (Key::W, false), (Key::E, true)])),
        &mut acks,
    );
    assert_eq!((served.injected, served.stopped), (3, None));
    assert_eq!(
        acks,
        [super::super::seat_socket_sink::ACK; 3],
        "one acknowledgement per injected transition"
    );
    // A transition the device refuses is not acknowledged.
    let mut refusing = SeatHelper::new(
        Keyboard {
            refuse_after: Some(0),
            ..Default::default()
        },
        &seat(),
    );
    let mut none = Vec::new();
    let served = refusing.serve(&mut Cursor::new(stream(&[(Key::W, true)])), &mut none);
    assert_eq!(served.stopped, Some("InjectionFailed"));
    assert!(none.is_empty());
}
