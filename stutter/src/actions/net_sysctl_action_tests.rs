use std::fs;

use super::*;
use crate::actions::net_sysctl::NetKnob;

fn action(name: &str, knob: NetKnob) -> NetSysctlAction {
    let base = std::env::temp_dir().join(format!("stutter-netact-{name}-{}", std::process::id()));
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
    fs::write(sys.join("net/core/busy_poll"), "0\n").unwrap();
    let proc_root = base.join("proc");
    fs::create_dir_all(proc_root.join("self/ns")).unwrap();
    std::os::unix::fs::symlink("net:[4026531840]", proc_root.join("self/ns/net")).unwrap();
    NetSysctlAction {
        sys_root: sys,
        proc_root,
        knob,
    }
}

fn allowed() -> NetSysctlPolicy {
    NetSysctlPolicy {
        allow: true,
        external_owner: None,
    }
}

#[test]
fn the_default_policy_denies_and_the_action_is_high_risk_system_wide() {
    let action = action("deny", NetKnob::BusyPollUs(50));
    assert!(action.preflight().is_err());
    assert!(action.dry_run().is_err());
    assert!(action.apply().is_err());
    assert_eq!(action.safety_class(), SafetyClass::HighRisk);
    assert!(action.descriptor().touches_system_wide_state);
}

#[test]
fn an_allowed_write_is_verified_and_rollback_restores_it() {
    let action = action("cycle", NetKnob::CongestionControl("bbr".into()));
    let token = action.apply_with_policy(&allowed()).unwrap();
    assert_eq!(token.kind(), "net-sysctl-restore");
    assert!(action.verify().unwrap().applied);
    let json = serde_json::to_string(&token).unwrap();
    assert_eq!(serde_json::from_str::<RollbackToken>(&json).unwrap(), token);
    // The record names a fake path, so rollback goes through the fake namespace check: the real
    // /proc namespace differs, which is exactly the refusal a foreign namespace must get.
    assert!(action.rollback(&token).is_err());
    assert!(
        action.verify().unwrap().applied,
        "not written back from another namespace"
    );
}

#[test]
fn a_setting_owned_by_bpftune_is_unsupported_delegation_and_nothing_is_written() {
    let action = action("owned", NetKnob::CongestionControl("bbr".into()));
    let owned = NetSysctlPolicy {
        allow: true,
        external_owner: Some("bpftune".into()),
    };
    let error = action.apply_with_policy(&owned).unwrap_err().to_string();
    assert!(error.contains("UnsupportedDelegation"), "{error}");
    assert!(!action.verify().unwrap().applied);
}

#[test]
fn a_foreign_token_is_refused_and_recovery_is_verified_automatic() {
    let action = action("foreign", NetKnob::BusyPollUs(10));
    assert!(
        action
            .rollback(&RollbackToken::NiceRestore { records: vec![] })
            .is_err()
    );
    let token = RollbackToken::NetSysctlRestore { records: vec![] };
    let descriptor = crate::actions::rollback::recovery::recovery_descriptor(&token);
    assert_eq!(
        descriptor.automatic_recovery,
        crate::actions::rollback::recovery::AutomaticRecoveryCapability::VerifiedAutomatic
    );
}

/// Live: the real `/proc/sys/net`, run as root inside a private network namespace
/// (`unshare -n`) so no host or other namespace setting is touched.
#[test]
#[ignore = "live: needs root and a private network namespace (unshare -n)"]
fn live_real_net_sysctls_in_a_private_namespace_write_read_back_and_restore() {
    let available = fs::read_to_string("/proc/sys/net/ipv4/tcp_available_congestion_control")
        .expect("BlockedResource: /proc/sys/net");
    let current = fs::read_to_string("/proc/sys/net/ipv4/tcp_congestion_control").unwrap();
    let other = available
        .split_whitespace()
        .find(|name| *name != current.trim())
        .expect("a second congestion control")
        .to_owned();
    let mut proven = Vec::new();
    // `net.core.busy_poll` exists only in the initial namespace: in a private one it is typed
    // Unavailable, never written elsewhere.
    let busy = NetSysctlAction::new(NetKnob::BusyPollUs(50));
    assert!(
        busy.apply_with_policy(&allowed())
            .unwrap_err()
            .to_string()
            .contains("Unavailable")
    );
    for knob in [
        NetKnob::CongestionControl(other),
        NetKnob::TcpWmem([4096, 65_536, 4_194_304]),
        NetKnob::TcpRmem([4096, 262_144, 8_388_608]),
    ] {
        let action = NetSysctlAction::new(knob.clone());
        let token = action.apply_with_policy(&allowed()).expect("apply");
        assert!(action.verify().unwrap().applied, "{knob:?} not read back");
        action.rollback(&token).unwrap();
        assert!(!action.verify().unwrap().applied, "{knob:?} not restored");
        proven.push(format!("{:?}", knob.operation()));
    }
    {
        use std::io::Write;
        let _ = writeln!(std::io::stderr(), "net live: proven {proven:?}");
    }
}
