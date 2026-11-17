use std::os::{fd::AsFd, unix::fs::PermissionsExt};

use super::*;

fn uid() -> u32 {
    crate::syscall::geteuid()
}

/// A private directory holding a real listening socket and held by a live "controller" (a flock on the
/// directory, as `ResearchServer::bind` takes).
struct Server {
    dir: tempfile::TempDir,
    _listener: std::os::unix::net::UnixListener,
    held: Option<std::fs::File>,
}

fn server() -> Server {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let path = dir.path().join(SOCKET_NAME);
    let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let held = std::fs::File::open(dir.path()).unwrap();
    assert!(crate::syscall::flock_exclusive_nonblocking(held.as_fd()));
    Server {
        dir,
        _listener: listener,
        held: Some(held),
    }
}

#[test]
fn a_private_socket_in_a_private_directory_held_by_a_live_controller_has_no_blockers() {
    let s = server();
    let facts = inspect(s.dir.path(), uid()).unwrap();
    assert_eq!(
        facts,
        ControlSocketFacts {
            directory_private: true,
            socket_private: true,
            controller_holds_directory: Some(true)
        }
    );
    assert!(blockers(Some(facts)).is_empty());
}

#[test]
fn each_way_the_socket_can_be_reachable_by_anyone_else_is_its_own_blocker() {
    // A directory others can enter, a socket others can write, and a plain file where the socket belongs.
    let s = server();
    std::fs::set_permissions(s.dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        blockers(inspect(s.dir.path(), uid())),
        ["ControlDirectoryNotPrivate"]
    );
    let s = server();
    std::fs::set_permissions(
        s.dir.path().join(SOCKET_NAME),
        std::fs::Permissions::from_mode(0o666),
    )
    .unwrap();
    assert_eq!(
        blockers(inspect(s.dir.path(), uid())),
        ["ControlSocketNotPrivate"]
    );
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(dir.path().join(SOCKET_NAME), b"not a socket").unwrap();
    assert!(blockers(inspect(dir.path(), uid())).contains(&"ControlSocketNotPrivate"));
    // A symbolic link standing in for the socket is not the socket.
    let linked = tempfile::tempdir().unwrap();
    std::fs::set_permissions(linked.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    std::os::unix::fs::symlink("/etc/passwd", linked.path().join(SOCKET_NAME)).unwrap();
    assert!(blockers(inspect(linked.path(), uid())).contains(&"ControlSocketNotPrivate"));
    // Owned by another user's uid as seen by the inspection.
    assert!(
        !inspect(server().dir.path(), uid().wrapping_add(1))
            .unwrap()
            .directory_private
    );
}

#[test]
fn a_socket_nobody_holds_and_an_absent_directory_are_not_a_running_controller() {
    let mut s = server();
    drop(s.held.take());
    assert_eq!(
        blockers(inspect(s.dir.path(), uid())),
        ["NoControllerHoldsTheControlDirectory"]
    );
    assert_eq!(
        blockers(inspect(Path::new("/nonexistent/stutter"), uid())),
        ["ControlSocketNotInspected"]
    );
}

/// Read-only inspection of the live resident controller's socket:
/// `STUTTER_REAL_SOCKET_DIR=/run/stutter-autotune-p2949 cargo test real_control_socket -- --ignored --nocapture`.
#[test]
#[ignore = "needs the resident controller's socket directory"]
fn real_control_socket() {
    let dir = std::env::var_os("STUTTER_REAL_SOCKET_DIR").expect("set STUTTER_REAL_SOCKET_DIR");
    let facts = inspect(Path::new(&dir), uid());
    println!("{facts:?}");
    assert!(blockers(facts).is_empty(), "{facts:?}");
}
