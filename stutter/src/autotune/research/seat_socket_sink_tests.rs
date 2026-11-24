use std::{
    io::Read,
    os::unix::{fs::PermissionsExt, net::UnixListener},
};

use super::*;
use crate::autotune::research::{
    route_driver::RouteInput,
    scripted_input::{
        Key, KeyStep, PhaseScript, ScriptedInput,
        isolated_seat::{IsolatedSink, LiveSession, attest},
    },
};

fn socket(dir: &tempfile::TempDir, mode: u32) -> (std::path::PathBuf, UnixListener) {
    let path = dir.path().join("seat.sock");
    let listener = UnixListener::bind(&path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
    (path, listener)
}

#[test]
fn only_the_expected_users_private_socket_is_connected_to() {
    let dir = tempfile::tempdir().unwrap();
    let uid = crate::syscall::geteuid();
    let (path, _listener) = socket(&dir, 0o600);
    assert!(SeatSocketSink::connect(&path, uid, "wayland-77").is_ok());
    // Another user's socket, a socket anyone can write, a path that is not a socket, a missing one, a link to one.
    assert_eq!(
        SeatSocketSink::connect(&path, uid + 1, "wayland-77").err(),
        Some("SinkSocketNotPrivate")
    );
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
    assert_eq!(
        SeatSocketSink::connect(&path, uid, "wayland-77").err(),
        Some("SinkSocketNotPrivate")
    );
    let plain = dir.path().join("plain");
    std::fs::write(&plain, b"x").unwrap();
    assert_eq!(
        SeatSocketSink::connect(&plain, uid, "wayland-77").err(),
        Some("SinkSocketNotASocket")
    );
    assert_eq!(
        SeatSocketSink::connect(&dir.path().join("absent"), uid, "wayland-77").err(),
        Some("SinkSocketMissing")
    );
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let link = dir.path().join("link.sock");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    assert_eq!(
        SeatSocketSink::connect(&link, uid, "wayland-77").err(),
        Some("SinkSocketNotASocket")
    );
    // A sink that names no display is never bound to a seat.
    assert_eq!(
        SeatSocketSink::connect(&path, uid, "").err(),
        Some("SinkDisplayUnnamed")
    );
}

#[test]
fn a_route_reaches_the_helper_only_through_an_attested_seat_whole_and_a_dead_helper_fails_the_write()
 {
    let dir = tempfile::tempdir().unwrap();
    let uid = crate::syscall::geteuid();
    let (path, listener) = socket(&dir, 0o600);
    let live = LiveSession {
        wayland_display: "wayland-1".into(),
        x_display: ":0".into(),
        session_id: "11".into(),
    };
    let game =
        b"WAYLAND_DISPLAY=wayland-77\0DISPLAY=:77\0XDG_SESSION_ID=42\0XDG_SEAT=seat7\0".to_vec();
    let seat = attest(&game, &live).unwrap();
    // A helper bound to the live display is refused by the seat gate before any event is written.
    let on_live = SeatSocketSink::connect(&path, uid, "wayland-1").unwrap();
    assert_eq!(
        IsolatedSink::new(on_live, &seat).err(),
        Some("SinkNotOnTheIsolatedSeat")
    );
    // Bound to the attested seat, the scripted route's events arrive whole at the helper, and every key is released.
    let sink = IsolatedSink::new(
        SeatSocketSink::connect(&path, uid, "wayland-77").unwrap(),
        &seat,
    )
    .unwrap();
    // The first connection was the refused sink's; the helper of the real one is the second.
    let (_refused, _) = listener.accept().unwrap();
    let (mut helper, _) = listener.accept().unwrap();
    let script = vec![PhaseScript {
        phase: "travel".into(),
        steps: vec![
            KeyStep {
                at_ms: 0,
                key: Key::W,
                down: true,
            },
            KeyStep {
                at_ms: 100,
                key: Key::W,
                down: false,
            },
        ],
    }];
    let mut input = ScriptedInput::new(sink, script);
    assert!(input.available());
    input.begin_phase("travel").unwrap();
    input.tick(200).unwrap();
    input.stop();
    assert!(input.released());
    drop(input);
    let mut received = Vec::new();
    helper.read_to_end(&mut received).unwrap();
    assert!(!received.is_empty() && received.len().is_multiple_of(EVENT_BYTES));
    // A helper that went away is a failed write, reported, never silently dropped.
    let dir2 = tempfile::tempdir().unwrap();
    let (path2, listener2) = socket(&dir2, 0o600);
    let mut sink = SeatSocketSink::connect(&path2, uid, "wayland-77").unwrap();
    let (helper2, _) = listener2.accept().unwrap();
    drop(helper2);
    let events = [0u8; EVENT_BYTES];
    let mut failed = false;
    for _ in 0..64 {
        if sink.write(&events).is_err() {
            failed = true;
            break;
        }
    }
    assert!(failed, "writes to a closed helper fail");
    // Torn or empty writes are refused before they are sent.
    let dir3 = tempfile::tempdir().unwrap();
    let (path3, _listener3) = socket(&dir3, 0o600);
    let mut sink = SeatSocketSink::connect(&path3, uid, "wayland-77").unwrap();
    assert_eq!(sink.write(&[]), Err("SinkEventsNotWhole"));
    assert_eq!(
        sink.write(&[0u8; EVENT_BYTES - 1]),
        Err("SinkEventsNotWhole")
    );
}

#[test]
fn the_helper_decodes_exactly_what_a_route_may_send_and_refuses_everything_else() {
    use crate::autotune::research::scripted_input::{Key, encode_key};
    let mut stream = Vec::new();
    stream.extend_from_slice(&encode_key(Key::W, true));
    stream.extend_from_slice(&encode_key(Key::W, false));
    assert_eq!(
        decode(&stream).unwrap(),
        vec![(Key::W, true), (Key::W, false)]
    );
    assert_eq!(decode(&[]).unwrap(), Vec::new());
    // A torn stream, a key a route never uses, a relative-motion event, and a value that is not press or release.
    assert_eq!(decode(&stream[..stream.len() - 1]), Err("SinkStreamTorn"));
    let mut other_key = encode_key(Key::W, true);
    other_key[18..20].copy_from_slice(&200u16.to_le_bytes());
    assert_eq!(decode(&other_key), Err("SinkEventRefused"));
    let mut motion = encode_key(Key::W, true);
    motion[16..18].copy_from_slice(&2u16.to_le_bytes());
    assert_eq!(decode(&motion), Err("SinkEventRefused"));
    let mut repeat = encode_key(Key::W, true);
    repeat[20..24].copy_from_slice(&2i32.to_le_bytes());
    assert_eq!(decode(&repeat), Err("SinkEventRefused"));
    let mut no_sync = encode_key(Key::W, true);
    no_sync[16 + EVENT_BYTES..18 + EVENT_BYTES].copy_from_slice(&1u16.to_le_bytes());
    assert_eq!(decode(&no_sync), Err("SinkEventRefused"));
}
