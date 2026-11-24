use super::*;
use crate::autotune::research::scripted_input::{
    EVENT_BYTES,
    isolated_seat::{LiveSession, attest},
};

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

#[test]
fn the_request_numbers_are_the_kernels_for_a_64_bit_little_endian_host() {
    // _IO('U', 1), _IO('U', 2), _IOW('U', 3, 92 bytes), _IOW('U', 100, int), _IOW('U', 101, int).
    let io = |nr: u64| (u64::from(b'U') << 8) | nr;
    let iow = |nr: u64, size: u64| (1 << 30) | (size << 16) | io(nr);
    assert_eq!(
        crate::syscall::uinput_requests(),
        [io(1), io(2), iow(3, 92), iow(100, 4), iow(101, 4)]
    );
}

#[test]
fn a_device_with_no_seat_or_on_another_seat_is_never_written_to() {
    // No ID_SEAT means seat0: the live seat.
    assert_eq!(udev_seat("E:ID_INPUT=1\nE:ID_INPUT_KEY=1\n"), "seat0");
    assert_eq!(udev_seat("E:ID_INPUT=1\nE:ID_SEAT=seat7\n"), "seat7");
    // The seat is the attested session's own (seat7): a device must be on exactly that one.
    let refused =
        |record: &str| DeviceInjector::new(Vec::<u8>::new(), &seat_for_test(), record).err();
    assert_eq!(
        refused("E:ID_INPUT=1\n"),
        Some("DeviceNotOnTheIsolatedSeat")
    );
    assert_eq!(
        refused("E:ID_SEAT=seat0\n"),
        Some("DeviceNotOnTheIsolatedSeat")
    );
    assert_eq!(
        refused("E:ID_SEAT=seat8\n"),
        Some("DeviceNotOnTheIsolatedSeat")
    );
    assert!(DeviceInjector::new(Vec::<u8>::new(), &seat_for_test(), "E:ID_SEAT=seat7\n").is_ok());
}

/// The seat comes from the game's own session, so no caller can name a seat of its choosing, and a game on `seat0` (or on
/// no seat the kernel's session shows) can never be attested at all.
#[test]
fn a_game_on_the_live_seat_or_on_none_is_never_attested() {
    let live = LiveSession {
        wayland_display: "wayland-1".into(),
        x_display: ":0".into(),
        session_id: "11".into(),
    };
    let game = |seat: &str| {
        format!("WAYLAND_DISPLAY=wayland-77\0DISPLAY=:77\0XDG_SESSION_ID=42\0{seat}").into_bytes()
    };
    assert_eq!(
        attest(&game("XDG_SEAT=seat0\0"), &live).err(),
        Some("GameOnTheLiveSeat")
    );
    assert_eq!(attest(&game(""), &live).err(), Some("GameSeatUnknown"));
    assert_eq!(
        attest(&game("XDG_SEAT=\0"), &live).err(),
        Some("GameSeatUnknown")
    );
    assert_eq!(
        attest(&game("XDG_SEAT=seat7\0"), &live).unwrap().seat(),
        "seat7"
    );
}

fn seat_for_test() -> IsolatedSeat {
    seat()
}

#[test]
fn a_device_on_the_isolated_seat_receives_whole_events_through_the_helper() {
    use crate::autotune::research::scripted_input::seat_helper::SeatHelper;
    let written = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    struct Shared(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl Write for Shared {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let injector =
        DeviceInjector::new(Shared(written.clone()), &seat(), "E:ID_SEAT=seat7\n").unwrap();
    let mut helper = SeatHelper::new(injector, &seat());
    let mut route = Vec::new();
    route.extend_from_slice(&encode_key(Key::W, true));
    route.extend_from_slice(&encode_key(Key::W, false));
    route.extend_from_slice(&encode_key(Key::E, true));
    let served = helper.serve(&mut std::io::Cursor::new(route));
    assert_eq!(
        (served.injected, served.stopped, served.all_released),
        (3, None, true)
    );
    // Two transitions of the route, the held key's release at the end: every record is a whole key plus sync pair.
    let bytes = written.lock().unwrap().clone();
    assert_eq!(bytes.len(), 4 * 2 * EVENT_BYTES);
    assert_eq!(
        super::super::seat_socket_sink::decode(&bytes)
            .unwrap()
            .last(),
        Some(&(Key::E, false))
    );
}

#[test]
fn creating_a_device_on_a_node_that_is_not_there_fails_without_touching_anything() {
    // The only kernel-touching function, exercised only on a path that cannot open: no device is ever registered here.
    assert_eq!(
        UinputDevice::create(std::path::Path::new("/nonexistent/uinput"), &seat()).err(),
        Some("UinputUnavailable")
    );
}
