use std::fs;

use super::*;

struct Fake {
    sys: PathBuf,
    proc_root: PathBuf,
}

fn fake(name: &str) -> Fake {
    let base = std::env::temp_dir().join(format!("stutter-net-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    let sys = base.join("sys");
    fs::create_dir_all(sys.join("net/ipv4")).unwrap();
    fs::create_dir_all(sys.join("net/core")).unwrap();
    fs::write(sys.join("net/ipv4/tcp_congestion_control"), "cubic\n").unwrap();
    fs::write(
        sys.join("net/ipv4/tcp_available_congestion_control"),
        "reno cubic bbr\n",
    )
    .unwrap();
    fs::write(sys.join("net/ipv4/tcp_rmem"), "4096\t131072\t6291456\n").unwrap();
    fs::write(sys.join("net/ipv4/tcp_wmem"), "4096\t16384\t4194304\n").unwrap();
    fs::write(sys.join("net/core/busy_poll"), "0\n").unwrap();
    let proc_root = base.join("proc");
    fs::create_dir_all(proc_root.join("self/ns")).unwrap();
    std::os::unix::fs::symlink("net:[4026531840]", proc_root.join("self/ns/net")).unwrap();
    Fake { sys, proc_root }
}

fn read(fake: &Fake, file: &str) -> String {
    fs::read_to_string(fake.sys.join(file))
        .unwrap()
        .trim()
        .to_owned()
}

#[test]
fn each_of_the_three_operations_prepares_commits_and_restores() {
    for (knob, file, applied, original) in [
        (
            NetKnob::CongestionControl("bbr".into()),
            "net/ipv4/tcp_congestion_control",
            "bbr",
            "cubic",
        ),
        (
            NetKnob::TcpRmem([4096, 262144, 8388608]),
            "net/ipv4/tcp_rmem",
            "4096 262144 8388608",
            "4096\t131072\t6291456",
        ),
        (NetKnob::BusyPollUs(50), "net/core/busy_poll", "50", "0"),
    ] {
        let fake = fake(file.rsplit('/').next().unwrap());
        let prepared = prepare(&fake.sys, &fake.proc_root, &knob, None).unwrap();
        assert_eq!(read(&fake, file), original, "prepare writes nothing");
        let committed = prepared.commit().unwrap();
        assert_eq!(norm(&read(&fake, file)), applied);
        assert_eq!(
            restore_recorded(&fake.proc_root, &committed.record()),
            NetRestore::Restored,
            "{file}"
        );
        assert_eq!(norm(&read(&fake, file)), norm(original));
    }
}

#[test]
fn the_three_operations_are_distinct() {
    assert_eq!(
        NetKnob::CongestionControl("x".into()).operation(),
        NetOperation::TcpCongestionControl
    );
    assert_eq!(
        NetKnob::TcpRmem([1, 2, 3]).operation(),
        NetOperation::TcpBuffers
    );
    assert_eq!(
        NetKnob::TcpWmem([1, 2, 3]).operation(),
        NetOperation::TcpBuffers
    );
    assert_eq!(NetKnob::BusyPollUs(1).operation(), NetOperation::BusyPoll);
}

#[test]
fn out_of_domain_values_owned_settings_and_no_ops_are_refused_before_any_write() {
    let fake = fake("deny");
    let deny =
        |knob: NetKnob, owner| prepare(&fake.sys, &fake.proc_root, &knob, owner).unwrap_err();
    assert!(matches!(
        deny(NetKnob::CongestionControl("vegas".into()), None),
        NetDenial::OutOfDomain(_)
    ));
    assert!(matches!(
        deny(NetKnob::TcpRmem([4096, 1024, 8192]), None),
        NetDenial::OutOfDomain(_)
    ));
    assert!(matches!(
        deny(NetKnob::TcpWmem([4096, 8192, 1 << 40]), None),
        NetDenial::OutOfDomain(_)
    ));
    assert!(matches!(
        deny(NetKnob::BusyPollUs(5000), None),
        NetDenial::OutOfDomain(_)
    ));
    assert_eq!(
        deny(NetKnob::CongestionControl("bbr".into()), Some("bpftune")),
        NetDenial::UnsupportedDelegation("bpftune".into())
    );
    assert_eq!(
        deny(NetKnob::CongestionControl("cubic".into()), None),
        NetDenial::NoChange
    );
    assert_eq!(read(&fake, "net/ipv4/tcp_congestion_control"), "cubic");
}

#[test]
fn restore_refuses_another_namespace_and_a_foreign_value() {
    let fake = fake("restore");
    let committed = prepare(&fake.sys, &fake.proc_root, &NetKnob::BusyPollUs(50), None)
        .unwrap()
        .commit()
        .unwrap();
    let record = committed.record();
    fs::write(fake.sys.join("net/core/busy_poll"), "80").unwrap();
    assert_eq!(
        restore_recorded(&fake.proc_root, &record),
        NetRestore::ExternallyChanged
    );
    assert_eq!(read(&fake, "net/core/busy_poll"), "80");
    fs::write(fake.sys.join("net/core/busy_poll"), "50").unwrap();
    fs::remove_file(fake.proc_root.join("self/ns/net")).unwrap();
    std::os::unix::fs::symlink("net:[4026532000]", fake.proc_root.join("self/ns/net")).unwrap();
    assert_eq!(
        restore_recorded(&fake.proc_root, &record),
        NetRestore::UnsafeToRestore
    );
    assert_eq!(
        read(&fake, "net/core/busy_poll"),
        "50",
        "another namespace is never written"
    );
}
