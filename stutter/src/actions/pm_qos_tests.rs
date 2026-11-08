use std::{
    fs,
    time::{Duration, Instant},
};

use super::*;

fn ac() -> PowerContext {
    PowerContext {
        on_ac: Some(true),
        throttle_events_recent: Some(0),
    }
}

fn device(name: &str, initial: u32) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("stutter-pmqos-{name}-{}", std::process::id()));
    fs::write(&path, initial.to_ne_bytes()).unwrap();
    path
}

#[test]
fn the_policy_enforces_a_floor_a_ceiling_and_power_guards_that_fail_closed() {
    let policy = PmQosPolicy::default();
    assert_eq!(policy.permit(50, &ac()), Ok(()));
    assert_eq!(
        policy.permit(0, &ac()),
        Err(PmQosDenial::BelowFloor),
        "zero keeps every CPU polling"
    );
    assert_eq!(policy.permit(9, &ac()), Err(PmQosDenial::BelowFloor));
    assert_eq!(policy.permit(10_001, &ac()), Err(PmQosDenial::AboveCeiling));
    let battery = PowerContext {
        on_ac: Some(false),
        ..ac()
    };
    assert_eq!(policy.permit(50, &battery), Err(PmQosDenial::NotOnAc));
    let unknown_power = PowerContext {
        on_ac: None,
        ..ac()
    };
    assert_eq!(policy.permit(50, &unknown_power), Err(PmQosDenial::NotOnAc));
    let hot = PowerContext {
        throttle_events_recent: Some(3),
        ..ac()
    };
    assert_eq!(policy.permit(50, &hot), Err(PmQosDenial::ThermalThrottling));
    let blind = PowerContext {
        throttle_events_recent: None,
        ..ac()
    };
    assert_eq!(
        policy.permit(50, &blind),
        Err(PmQosDenial::ThrottlingUnknown)
    );
    let lax = PmQosPolicy {
        require_ac: false,
        forbid_thermal_throttling: false,
        ..policy
    };
    assert_eq!(lax.permit(50, &battery), Ok(()));
}

#[test]
fn a_lease_writes_the_binary_request_verifies_it_and_expires_at_its_limit() {
    let path = device("ok", 0);
    let policy = PmQosPolicy {
        max_hold: Duration::from_millis(40),
        ..PmQosPolicy::default()
    };
    let lease = PmQosLease::acquire(&path, 50, &policy, &ac()).unwrap();
    assert_eq!(lease.requested_us, 50);
    assert_eq!(
        read_effective(&path).unwrap(),
        50,
        "four native-endian bytes, as the kernel expects"
    );
    assert!(!lease.expired(Instant::now()));
    assert!(lease.expired(Instant::now() + Duration::from_millis(50)));
    fs::remove_file(&path).unwrap();
}

#[test]
fn a_request_the_aggregate_does_not_reflect_is_released_not_kept() {
    let path = device("lost", 0);
    // The aggregate is the minimum over requesters. One above ours means our write did not take
    // effect, so the lease is refused and released.
    let ignored =
        PmQosLease::acquire_verified(&path, 50, &PmQosPolicy::default(), &ac(), || Ok(200));
    assert!(matches!(
        ignored,
        Err(LeaseError::NotInEffect {
            requested: 50,
            effective: 200
        })
    ));
    // A stricter requester (lower aggregate) is fine: ours is satisfied by theirs.
    let stricter =
        PmQosLease::acquire_verified(&path, 50, &PmQosPolicy::default(), &ac(), || Ok(5));
    assert!(stricter.is_ok());
    fs::remove_file(&path).unwrap();
}

#[test]
fn a_denied_request_never_touches_the_device() {
    let path = device("denied", 7);
    let battery = PowerContext {
        on_ac: Some(false),
        ..ac()
    };
    assert!(matches!(
        PmQosLease::acquire(&path, 50, &PmQosPolicy::default(), &battery),
        Err(LeaseError::Denied(PmQosDenial::NotOnAc))
    ));
    assert_eq!(read_effective(&path).unwrap(), 7, "unchanged");
    assert!(read_effective(std::path::Path::new("/nonexistent/cpu_dma_latency")).is_err());
    fs::remove_file(&path).unwrap();
}
