use std::{fs, time::Duration};

use super::*;

fn ac() -> PowerContext {
    PowerContext {
        on_ac: Some(true),
        throttle_events_recent: Some(0),
    }
}

fn device(name: &str) -> PathBuf {
    let path =
        std::env::temp_dir().join(format!("stutter-pmqos-act-{name}-{}", std::process::id()));
    fs::write(&path, 2_000_000_u32.to_ne_bytes()).unwrap();
    path
}

fn policy(hold: Duration) -> PmQosActionPolicy {
    PmQosActionPolicy {
        allow: true,
        lease: PmQosPolicy {
            max_hold: hold,
            ..PmQosPolicy::default()
        },
        power: ac(),
    }
}

fn action(device: PathBuf) -> PmQosCpuLatencyAction {
    PmQosCpuLatencyAction {
        device,
        latency_us: 50,
    }
}

#[test]
fn the_default_policy_denies_everything_and_the_action_is_high_risk_system_wide() {
    let action = action(device("deny"));
    assert!(action.preflight().is_err());
    assert!(action.dry_run().is_err());
    assert!(
        action.apply().is_err(),
        "the default apply path writes nothing"
    );
    assert_eq!(action.safety_class(), SafetyClass::HighRisk);
    let descriptor = action.descriptor();
    assert!(descriptor.touches_system_wide_state);
    assert_eq!(
        descriptor.effect_scope,
        crate::daemon_policy::ActionEffectScope::SystemWide
    );
}

#[test]
fn an_explicitly_allowed_apply_holds_a_lease_and_rollback_releases_it() {
    let device = device("cycle");
    let action = action(device.clone());
    let token = action
        .apply_with_policy(&policy(Duration::from_secs(60)))
        .unwrap();
    assert_eq!(token.kind(), "pm-qos-release");
    assert!(action.verify().unwrap().applied);
    // The kernel stand-in drops the request once its descriptor closes.
    let holders_before = HELD.lock().unwrap().len();
    fs::write(&device, 2_000_000_u32.to_ne_bytes()).unwrap();
    action.rollback(&token).unwrap();
    assert_eq!(HELD.lock().unwrap().len(), holders_before - 1);
}

#[test]
fn power_guards_block_an_allowed_apply_without_touching_the_device() {
    let device = device("battery");
    let mut battery = policy(Duration::from_secs(60));
    battery.power.on_ac = Some(false);
    assert!(action(device.clone()).apply_with_policy(&battery).is_err());
    assert_eq!(
        crate::actions::pm_qos::read_effective(&device).unwrap(),
        2_000_000
    );
}

#[test]
fn rollback_refuses_a_token_of_another_kind() {
    let action = action(device("kind"));
    let other = RollbackToken::NiceRestore { records: vec![] };
    assert!(action.rollback(&other).is_err());
}

/// Live: the real `/dev/cpu_dma_latency`, run as root. Takes a 1 ms lease for a moment, reads the
/// aggregate target while held, releases it through rollback and reads it again.
#[test]
#[ignore = "live: needs root and the real /dev/cpu_dma_latency"]
fn live_real_device_lease_changes_the_aggregate_target_and_rollback_releases_it() {
    let device = PathBuf::from(PM_QOS_DEVICE);
    let before = crate::actions::pm_qos::read_effective(&device).expect("BlockedResource: root");
    let action = PmQosCpuLatencyAction {
        device: device.clone(),
        latency_us: 1000,
    };
    let token = action
        .apply_with_policy(&policy(Duration::from_secs(5)))
        .unwrap();
    let held = crate::actions::pm_qos::read_effective(&device).unwrap();
    action.rollback(&token).unwrap();
    let after = crate::actions::pm_qos::read_effective(&device).unwrap();
    {
        use std::io::Write;
        let _ = writeln!(
            std::io::stderr(),
            "pm_qos live: before={before} held={held} after={after}"
        );
    }
    assert!(held <= 1000, "the lease did not take effect: {held}");
    assert_eq!(after, before, "the aggregate target did not return");
}
