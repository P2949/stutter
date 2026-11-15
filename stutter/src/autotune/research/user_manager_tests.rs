//! Tests for the user manager and bus peer attestation.

use std::os::unix::net::UnixStream;

use super::*;
use crate::integrations::dbus::fake::{Reply, serve};

#[test]
fn broker_candidate_requires_a_unique_stable_owner_of_the_peer_endpoint() {
    use std::os::unix::fs::symlink;
    let proc = tempfile::tempdir().unwrap();
    for pid in [123u32, 124] {
        let dir = proc.path().join(pid.to_string());
        std::fs::create_dir_all(dir.join("fd")).unwrap();
        std::fs::write(dir.join("comm"), "dbus-broker\n").unwrap();
    }
    symlink("socket:[42]", proc.path().join("123/fd/4")).unwrap();
    let uid = crate::syscall::geteuid();
    let candidate =
        broker_candidate(proc.path(), uid, 42, &|pid, _| Ok(u64::from(pid.as_u32()))).unwrap();
    assert_eq!((candidate.0.as_u32(), candidate.1), (123, 123));
    let candidate = broker_candidate(proc.path(), uid, 42, &|pid, _| {
        anyhow::ensure!(
            pid.as_u32() == 123,
            "unrelated broker must not be inspected"
        );
        Ok(123)
    })
    .unwrap();
    assert_eq!(candidate.0.as_u32(), 123);
    assert!(broker_candidate(proc.path(), uid, 43, &|_, _| Ok(1)).is_err());
    let calls = std::sync::atomic::AtomicUsize::new(0);
    assert!(
        broker_candidate(proc.path(), uid, 42, &|_, _| {
            Ok(calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) as u64 + 1)
        })
        .is_err()
    );
    symlink("socket:[42]", proc.path().join("124/fd/4")).unwrap();
    assert!(broker_candidate(proc.path(), uid, 42, &|_, _| Ok(1)).is_err());
}

#[test]
#[ignore = "read-only observation of the current user's live session bus"]
fn live_session_bus_peer_has_a_stable_broker_candidate() {
    let uid = crate::syscall::geteuid();
    let stream = connect_stream(&PathBuf::from(format!("/run/user/{uid}/bus"))).unwrap();
    let peer = super::super::unix_diag::peer_inode(&stream).unwrap();
    let (pid, ticks) = broker_candidate(Path::new("/proc"), uid, peer, &inspect_bus_peer).unwrap();
    assert!(ticks > 0);
    assert_eq!(inspect_bus_peer(pid, uid).unwrap(), ticks);
    assert_eq!(super::super::unix_diag::peer_inode(&stream).unwrap(), peer);
    assert!(super::super::unix_diag::process_holds_peer(Path::new("/proc"), pid, peer).unwrap());
}

fn reply_string(value: &str) -> Reply {
    let mut writer = Writer::default();
    writer.string(value);
    Reply::Body("s".into(), writer.buf)
}

fn reply_u32(value: u32) -> Reply {
    let mut writer = Writer::default();
    writer.u32(value);
    Reply::Body("u".into(), writer.buf)
}

#[test]
fn manager_and_bus_must_be_in_the_exact_user_service_scope() {
    assert!(manager_cgroup(
        "0::/user.slice/user-1000.slice/user@1000.service/init.scope\n",
        1000
    ));
    assert!(!manager_cgroup(
        "0::/other/user-1000.slice/user@1000.service/init.scope\n",
        1000
    ));
    assert!(!manager_cgroup(
        "0::/user.slice/user-1001.slice/user@1001.service/init.scope\n",
        1000
    ));
    assert!(bus_cgroup(
        "0::/user.slice/user-1000.slice/user@1000.service/session.slice/dbus-broker.service\n",
        1000
    ));
    assert!(!bus_cgroup(
        "0::/user.slice/user-1000.slice/user@1000.service/app.service/dbus-broker.service\n",
        1000
    ));
    assert!(!bus_cgroup(
        "0::/other/user-1000.slice/user@1000.service/session.slice/dbus-broker.service\n",
        1000
    ));
    assert!(manager_command_line(b"/nix/store/systemd\0--user\0"));
    assert!(!manager_command_line(b"/nix/store/systemd\0--user"));
    assert!(!manager_command_line(
        b"/nix/store/systemd\0--user\0--user\0"
    ));
}

#[test]
fn owner_attestation_uses_only_read_only_queries_and_rechecks_identity() {
    let (client_end, server_end) = UnixStream::pair().unwrap();
    let calls = serve(server_end, |call| match call.member.as_str() {
        "GetNameOwner" => reply_string(":1.7"),
        "GetConnectionUnixUser" => reply_u32(1000),
        "GetConnectionUnixProcessID" => reply_u32(4242),
        _ => Reply::Error("org.test.Unexpected"),
    });
    let mut client = DbusClient::from_stream(client_end, 1000).unwrap();
    let identity = authenticate_manager(&mut client, 1000, &|pid, uid| {
        assert_eq!((pid.as_u32(), uid), (4242, 1000));
        Ok(77)
    })
    .unwrap();
    assert_eq!(identity.pid.as_u32(), 4242);
    assert_eq!(identity.start_ticks, 77);
    drop(client);
    let seen = calls.lock().unwrap();
    assert_eq!(
        seen.iter()
            .map(|call| call.member.as_str())
            .collect::<Vec<_>>(),
        [
            "GetNameOwner",
            "GetConnectionUnixUser",
            "GetConnectionUnixProcessID",
            "GetNameOwner",
            "GetConnectionUnixProcessID"
        ]
    );
}

#[test]
fn wrong_bus_owner_and_changed_owner_are_refused() {
    let (client_end, server_end) = UnixStream::pair().unwrap();
    serve(server_end, |call| match call.member.as_str() {
        "GetNameOwner" => reply_string(":1.7"),
        "GetConnectionUnixUser" => reply_u32(1001),
        _ => Reply::Error("org.test.Unexpected"),
    });
    let mut client = DbusClient::from_stream(client_end, 1000).unwrap();
    assert!(authenticate_manager(&mut client, 1000, &|_, _| Ok(77)).is_err());

    let (client_end, server_end) = UnixStream::pair().unwrap();
    let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = count.clone();
    serve(server_end, move |call| match call.member.as_str() {
        "GetNameOwner" => {
            if observed.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 0 {
                reply_string(":1.7")
            } else {
                reply_string(":1.8")
            }
        }
        "GetConnectionUnixUser" => reply_u32(1000),
        "GetConnectionUnixProcessID" => reply_u32(4242),
        _ => Reply::Error("org.test.Unexpected"),
    });
    let mut client = DbusClient::from_stream(client_end, 1000).unwrap();
    assert!(authenticate_manager(&mut client, 1000, &|_, _| Ok(77)).is_err());
}

#[test]
fn owner_reply_from_a_different_sender_is_refused() {
    let (client_end, server_end) = UnixStream::pair().unwrap();
    serve(server_end, |_| {
        let mut writer = Writer::default();
        writer.string(":1.7");
        Reply::BodyFrom("s".into(), writer.buf, ":1.666")
    });
    let mut client = DbusClient::from_stream(client_end, 1000).unwrap();
    assert!(authenticate_manager(&mut client, 1000, &|_, _| Ok(77)).is_err());
}

#[test]
fn manager_witness_expires_and_retains_its_identity() {
    let (client_end, server_end) = UnixStream::pair().unwrap();
    serve(server_end, |_| Reply::Error("org.test.Unexpected"));
    let peer_stream = client_end.try_clone().unwrap();
    let client = DbusClient::from_stream(client_end, 1000).unwrap();
    let identity = ManagerIdentity {
        uid: 1000,
        pid: Pid::new(4242),
        start_ticks: 77,
        unique_name: ":1.7".into(),
    };
    let mut manager = VerifiedUserManager {
        client,
        peer_stream,
        peer_inode: 1,
        identity: identity.clone(),
        peer_pid: Pid::new(4243),
        peer_start_ticks: 78,
        runtime: "/run/user/1000".into(),
        runtime_device: 1,
        runtime_inode: 2,
        socket_device: 3,
        socket_inode: 4,
        boot_id: "fixture-boot".into(),
        session_id: "fixture-session".into(),
        session_observed_at: Instant::now() - Duration::from_secs(4),
        verified_at: Instant::now() - Duration::from_secs(4),
    };
    assert_eq!(manager.identity(), &identity);
    assert!(!manager.is_fresh());
    assert!(manager.revalidate().is_err());
    let _: fn(
        &VerifiedSessionEnvironment,
        &ResearchIntentV1,
    ) -> anyhow::Result<VerifiedUserManager> = VerifiedUserManager::inspect;
}
