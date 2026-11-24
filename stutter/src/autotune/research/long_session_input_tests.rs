use std::{
    io::{Read, Write},
    os::unix::{fs::PermissionsExt, net::UnixListener},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use super::*;
use crate::autotune::research::{
    long_receipt::{LongProgress, LongSessionDriver},
    long_session::LongWorkUnit,
    scripted_input::{EVENT_BYTES, seat_socket_sink::ACK},
};

fn uid() -> u32 {
    crate::syscall::geteuid()
}

/// A scratch directory holding a save, a helper socket path and the session file.
struct Rig {
    dir: tempfile::TempDir,
}

impl Rig {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let save = dir.path().join("save");
        std::fs::create_dir(&save).unwrap();
        std::fs::write(save.join("manual.dat"), b"the save").unwrap();
        Self { dir }
    }

    fn save(&self) -> std::path::PathBuf {
        self.dir.path().join("save")
    }

    fn socket(&self) -> std::path::PathBuf {
        self.dir.path().join("helper.sock")
    }

    /// The session file for a two-phase route: travel (2 s) then fight (1 s).
    fn config(&self, edit: impl FnOnce(&mut serde_json::Value)) -> std::path::PathBuf {
        let mut value = serde_json::json!({
            "route": [{ "phase": "travel", "seconds": 2 }, { "phase": "fight", "seconds": 1 }],
            "script": [
                { "phase": "travel", "steps": [
                    { "at_ms": 0, "key": "w", "down": true }, { "at_ms": 1000, "key": "w", "down": false } ] },
                { "phase": "fight", "steps": [
                    { "at_ms": 0, "key": "e", "down": true }, { "at_ms": 500, "key": "e", "down": false } ] },
            ],
            "save_dir": self.save(), "helper_socket": self.socket(),
            "live": { "wayland_display": "wayland-1", "x_display": ":0", "session_id": "11" },
        });
        edit(&mut value);
        let path = self.dir.path().join("session.json");
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        path
    }

    /// A helper's socket that collects every byte it is sent until the sender closes.
    fn helper(&self) -> std::thread::JoinHandle<Vec<u8>> {
        let listener = UnixListener::bind(self.socket()).unwrap();
        std::fs::set_permissions(self.socket(), std::fs::Permissions::from_mode(0o600)).unwrap();
        std::thread::spawn(move || {
            let mut received = Vec::new();
            if let Ok((mut stream, _)) = listener.accept() {
                // Each key transition is acknowledged, as the real helper does after injecting it.
                let mut record = [0u8; 2 * EVENT_BYTES];
                while stream.read_exact(&mut record).is_ok() {
                    received.extend_from_slice(&record);
                    if stream.write_all(&[ACK]).is_err() {
                        break;
                    }
                }
            }
            received
        })
    }
}

/// A stand-in game: a process with the environment of a game on a session and seat of its own.
fn game(environment: &[(&str, &str)]) -> (std::process::Child, SessionProcess) {
    let child = std::process::Command::new("sleep")
        .arg("60")
        .env_clear()
        .envs(environment.iter().copied())
        .spawn()
        .unwrap();
    let start_ticks = crate::process_tree::process_starttime_at(Path::new("/proc"), child.id())
        .expect("the stand-in game is alive");
    let process = SessionProcess {
        pid: stutter_core::ids::Pid::new(child.id()),
        start_ticks,
        uid: uid(),
    };
    (child, process)
}

const OWN_SEAT: [(&str, &str); 4] = [
    ("WAYLAND_DISPLAY", "wayland-77"),
    ("DISPLAY", ":77"),
    ("XDG_SESSION_ID", "42"),
    ("XDG_SEAT", "seat7"),
];

fn driver_on(config: &Path, clock: &Arc<AtomicU64>) -> RouteDriver<SessionInput> {
    let loaded = load(config, uid()).unwrap();
    let clock = clock.clone();
    RouteDriver::new(
        loaded.input,
        loaded.route,
        Box::new(move || clock.load(Ordering::SeqCst)),
    )
    .with_save_dir(loaded.save_dir)
}

fn unit_for(driver: &RouteDriver<SessionInput>) -> LongWorkUnit {
    LongWorkUnit {
        id: "save-a-route-1".into(),
        duration_seconds: 3,
        definition_sha256: driver.definition_sha256(),
    }
}

/// The key events of a byte stream: (code, value) of every EV_KEY record.
fn keys(bytes: &[u8]) -> Vec<(u16, i32)> {
    bytes
        .chunks(EVENT_BYTES)
        .filter(|record| u16::from_le_bytes([record[16], record[17]]) == 1)
        .map(|record| {
            (
                u16::from_le_bytes([record[18], record[19]]),
                i32::from_le_bytes([record[20], record[21], record[22], record[23]]),
            )
        })
        .collect()
}

#[test]
fn a_session_file_that_is_not_the_users_private_valid_file_is_refused_with_its_typed_reason() {
    let rig = Rig::new();
    let refused = |config: std::path::PathBuf, why: &'static str| {
        assert_eq!(load(&config, uid()).err(), Some(why), "{config:?}");
    };
    // Group-writable, a link, absent, another user's, and one that is not JSON.
    let writable = rig.config(|_| {});
    std::fs::set_permissions(&writable, std::fs::Permissions::from_mode(0o660)).unwrap();
    refused(writable.clone(), "LongSessionConfigNotPrivate");
    std::fs::set_permissions(&writable, std::fs::Permissions::from_mode(0o600)).unwrap();
    let link = rig.dir.path().join("link.json");
    std::os::unix::fs::symlink(&writable, &link).unwrap();
    refused(link, "LongSessionConfigUnreadable");
    refused(
        rig.dir.path().join("absent.json"),
        "LongSessionConfigUnreadable",
    );
    assert_eq!(
        load(&writable, uid() + 1).err(),
        Some("LongSessionConfigNotPrivate")
    );
    std::fs::write(&writable, b"not json").unwrap();
    refused(writable, "LongSessionConfigInvalid");
    // A key the route may not press, a phase with no script, a zero-length step, an empty route and relative paths.
    refused(
        rig.config(|v| v["script"][0]["steps"][0]["key"] = "delete".into()),
        "LongSessionConfigInvalid",
    );
    refused(
        rig.config(|v| v["route"][1]["phase"] = "swim".into()),
        "LongSessionConfigInvalid",
    );
    refused(
        rig.config(|v| v["route"][0]["seconds"] = 0.into()),
        "LongSessionConfigInvalid",
    );
    refused(
        rig.config(|v| v["route"] = serde_json::json!([])),
        "LongSessionConfigInvalid",
    );
    refused(
        rig.config(|v| v["save_dir"] = "save".into()),
        "LongSessionConfigInvalid",
    );
    refused(
        rig.config(|v| v["extra"] = 1.into()),
        "LongSessionConfigInvalid",
    );
    // A valid one loads.
    assert!(load(&rig.config(|_| {}), uid()).is_ok());
}

#[test]
fn the_definition_a_unit_must_carry_binds_the_route_the_script_and_the_save_as_it_is_now() {
    let rig = Rig::new();
    let clock = Arc::new(AtomicU64::new(100));
    let config = rig.config(|_| {});
    let mut driver = driver_on(&config, &clock);
    let unit = unit_for(&driver);
    let loaded = load(&config, uid()).unwrap();
    let expected =
        crate::autotune::research::route_driver::save_identity::session_definition_sha256(
            &loaded.route,
            &loaded.input.script_sha256(),
            &crate::autotune::research::route_driver::save_identity::save_identity_sha256(
                &rig.save(),
            )
            .unwrap(),
        );
    assert_eq!(unit.definition_sha256, expected);
    assert_eq!(driver.start(&unit), Ok(()));
    // The save changed after the unit was authorized: it is another session, and it does not start.
    std::fs::write(rig.save().join("manual.dat"), b"another save").unwrap();
    let mut changed = driver_on(&config, &clock);
    assert_eq!(changed.start(&unit), Err("BlockedRouteDoesNotMatchTheUnit"));
    // A save that cannot be read has no definition at all.
    std::fs::remove_dir_all(rig.save()).unwrap();
    let mut gone = driver_on(&config, &clock);
    assert_eq!(gone.start(&unit), Err("BlockedSaveUnreadable"));
}

#[test]
fn the_route_is_played_through_the_attested_seat_and_every_key_ends_released() {
    let rig = Rig::new();
    let helper = rig.helper();
    let clock = Arc::new(AtomicU64::new(100));
    let mut driver = driver_on(&rig.config(|_| {}), &clock);
    let (mut child, process) = game(&OWN_SEAT);
    driver.start(&unit_for(&driver)).unwrap();
    driver.observe_process(Some(process));
    // The first poll binds the input to the game's seat and begins the first phase; nothing is sent before that.
    assert!(matches!(driver.poll(), LongProgress::Running));
    let mut finished = None;
    for second in 101..=103 {
        clock.store(second, Ordering::SeqCst);
        if let LongProgress::Completed(record) = driver.poll() {
            finished = Some(record);
            break;
        }
    }
    let record = finished.expect("the route completed");
    assert_eq!(
        record
            .phases
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["travel", "fight"]
    );
    assert!(driver.abort().is_ok());
    drop(driver);
    let received = helper.join().unwrap();
    assert!(!received.is_empty() && received.len().is_multiple_of(EVENT_BYTES));
    let sent = keys(&received);
    // W went down and up in travel; E went down in fight and was released when the route ended.
    assert_eq!(sent.first(), Some(&(17, 1)));
    assert!(sent.contains(&(17, 0)));
    assert_eq!(sent.last(), Some(&(18, 0)), "{sent:?}");
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn a_game_that_shares_the_live_display_or_seat_gets_nothing_and_fails_the_session_before_a_key() {
    let rig = Rig::new();
    let helper = rig.helper();
    let clock = Arc::new(AtomicU64::new(100));
    let attempt = |environment: &[(&str, &str)], why: &str| {
        let mut driver = driver_on(&rig.config(|_| {}), &clock);
        let (mut child, process) = game(environment);
        driver.start(&unit_for(&driver)).unwrap();
        driver.observe_process(Some(process));
        assert!(
            matches!(driver.poll(), LongProgress::Failed(code) if code == why),
            "{why}"
        );
        let _ = child.kill();
        let _ = child.wait();
    };
    // The live display, the live session and the live seat.
    attempt(
        &[
            ("WAYLAND_DISPLAY", "wayland-1"),
            ("DISPLAY", ":77"),
            ("XDG_SESSION_ID", "42"),
            ("XDG_SEAT", "seat7"),
        ],
        "GameSharesTheLiveDisplay",
    );
    attempt(
        &[
            ("WAYLAND_DISPLAY", "wayland-77"),
            ("DISPLAY", ":77"),
            ("XDG_SESSION_ID", "11"),
            ("XDG_SEAT", "seat7"),
        ],
        "GameSharesTheLiveSession",
    );
    attempt(
        &[
            ("WAYLAND_DISPLAY", "wayland-77"),
            ("DISPLAY", ":77"),
            ("XDG_SESSION_ID", "42"),
            ("XDG_SEAT", "seat0"),
        ],
        "GameOnTheLiveSeat",
    );
    // The helper was never connected to: it received nothing (the accept is released by closing the listener side).
    drop(std::os::unix::net::UnixStream::connect(rig.socket()));
    assert!(helper.join().unwrap().is_empty());
}

#[test]
fn a_missing_helper_or_a_game_that_is_no_longer_the_attested_process_fails_the_session() {
    let rig = Rig::new();
    let clock = Arc::new(AtomicU64::new(100));
    // No helper socket at all.
    let mut driver = driver_on(&rig.config(|_| {}), &clock);
    let (mut child, process) = game(&OWN_SEAT);
    driver.start(&unit_for(&driver)).unwrap();
    driver.observe_process(Some(process));
    assert!(matches!(
        driver.poll(),
        LongProgress::Failed("SinkSocketMissing")
    ));
    // A pid that is not the attested process any more (another start time).
    let helper = rig.helper();
    let mut other = driver_on(&rig.config(|_| {}), &clock);
    other.start(&unit_for(&other)).unwrap();
    other.observe_process(Some(SessionProcess {
        start_ticks: process.start_ticks + 1,
        ..process
    }));
    assert!(matches!(
        other.poll(),
        LongProgress::Failed("GameProcessChanged")
    ));
    drop(std::os::unix::net::UnixStream::connect(rig.socket()));
    assert!(helper.join().unwrap().is_empty());
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn the_authorization_the_operator_reads_is_exactly_the_definition_and_duration_a_unit_must_carry() {
    let rig = Rig::new();
    let clock = Arc::new(AtomicU64::new(100));
    let config = rig.config(|_| {});
    let authorization = authorization(&config, uid()).unwrap();
    let unit = unit_for(&driver_on(&config, &clock));
    assert_eq!(authorization["definition_sha256"], unit.definition_sha256);
    assert_eq!(authorization["duration_seconds"], 3);
    assert_eq!(
        authorization["phases"],
        serde_json::json!([{ "phase": "travel", "seconds": 2 }, { "phase": "fight", "seconds": 1 }])
    );
    // The save as it is now: changing it changes the definition the operator is told to authorize.
    std::fs::write(rig.save().join("manual.dat"), b"another save").unwrap();
    assert_ne!(
        super::authorization(&config, uid()).unwrap()["definition_sha256"],
        unit.definition_sha256
    );
    // A save that cannot be read, or a file that is not valid, is a typed refusal, never a digest.
    std::fs::remove_dir_all(rig.save()).unwrap();
    assert_eq!(
        super::authorization(&config, uid()).err(),
        Some("BlockedSaveUnreadable")
    );
    let bad = rig.config(|v| v["route"] = serde_json::json!([]));
    assert_eq!(
        super::authorization(&bad, uid()).err(),
        Some("LongSessionConfigInvalid")
    );
}
