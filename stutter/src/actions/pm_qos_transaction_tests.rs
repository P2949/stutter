use std::{fs, path::PathBuf, time::Duration};

use super::*;

fn ac() -> PowerContext {
    PowerContext {
        on_ac: Some(true),
        throttle_events_recent: Some(0),
    }
}

/// A fake `/dev/cpu_dma_latency`: a file holding the aggregate target the "kernel" reports.
fn fake_device(name: &str, aggregate: u32) -> PathBuf {
    let path = std::env::temp_dir().join(format!("stutter-pmqos-tx-{name}-{}", std::process::id()));
    fs::write(&path, aggregate.to_ne_bytes()).unwrap();
    path
}

fn set(path: &Path, value: u32) {
    fs::write(path, value.to_ne_bytes()).unwrap();
}

#[test]
fn prepare_reads_the_baseline_and_writes_nothing() {
    let device = fake_device("dry", 2_000_000);
    let prepared = prepare(&device, 50, &PmQosPolicy::default(), &ac());
    assert!(matches!(prepared, Ok(ref p) if p.baseline_us() == 2_000_000));
    assert_eq!(
        super::super::pm_qos::read_effective(&device).unwrap(),
        2_000_000
    );
}

#[test]
fn prepare_refuses_denied_unavailable_and_pointless_requests() {
    let device = fake_device("refuse", 2_000_000);
    let policy = PmQosPolicy::default();
    assert!(matches!(
        prepare(&device, 1, &policy, &ac()),
        Err(PrepareError::Denied(PmQosDenial::BelowFloor))
    ));
    let battery = PowerContext {
        on_ac: Some(false),
        ..ac()
    };
    assert!(matches!(
        prepare(&device, 50, &policy, &battery),
        Err(PrepareError::Denied(PmQosDenial::NotOnAc))
    ));
    let missing = std::env::temp_dir().join("stutter-pmqos-tx-absent-device");
    assert!(matches!(
        prepare(&missing, 50, &policy, &ac()),
        Err(PrepareError::Unavailable(_))
    ));
    set(&device, 20);
    assert!(matches!(
        prepare(&device, 50, &policy, &ac()),
        Err(PrepareError::AlreadySatisfied { effective_us: 20 })
    ));
}

#[test]
fn commit_holds_a_verified_lease_and_restore_proves_the_baseline_returned() {
    let device = fake_device("cycle", 2_000_000);
    let prepared = prepare(&device, 50, &PmQosPolicy::default(), &ac()).unwrap();
    let held = prepared.commit(&ac()).unwrap();
    assert_eq!(held.requested_us(), 50);
    assert!(!held.expired(Instant::now()));
    // The fake kernel drops the request when the descriptor closes.
    let outcome = held.restore_with(|| Ok(2_000_000));
    assert_eq!(
        outcome,
        ReleaseOutcome::Restored {
            effective_us: 2_000_000
        }
    );
}

#[test]
fn commit_rechecks_power_guards_with_fresh_facts() {
    let device = fake_device("fresh", 2_000_000);
    let prepared = prepare(&device, 50, &PmQosPolicy::default(), &ac()).unwrap();
    let hot = PowerContext {
        throttle_events_recent: Some(4),
        ..ac()
    };
    assert!(matches!(
        prepared.commit(&hot),
        Err(LeaseError::Denied(PmQosDenial::ThermalThrottling))
    ));
}

#[test]
fn another_requester_is_not_ours_to_remove_and_an_unread_target_is_unproven() {
    let device = fake_device("other", 2_000_000);
    let held = prepare(&device, 50, &PmQosPolicy::default(), &ac())
        .unwrap()
        .commit(&ac())
        .unwrap();
    assert_eq!(
        held.restore_with(|| Ok(500)),
        ReleaseOutcome::ReleasedOtherRequesterActive { effective_us: 500 }
    );
    let held = prepare(
        &device_with(2_000_000, "other2"),
        50,
        &PmQosPolicy::default(),
        &ac(),
    )
    .unwrap()
    .commit(&ac())
    .unwrap();
    assert_eq!(held.restore_with(|| Ok(50)), ReleaseOutcome::Unproven);
    let held = prepare(
        &device_with(2_000_000, "other3"),
        50,
        &PmQosPolicy::default(),
        &ac(),
    )
    .unwrap()
    .commit(&ac())
    .unwrap();
    assert_eq!(
        held.restore_with(|| Err(io::Error::other("gone"))),
        ReleaseOutcome::Unproven
    );
}

fn device_with(value: u32, name: &str) -> PathBuf {
    fake_device(name, value)
}

#[test]
fn a_lease_past_its_hold_limit_is_reported_expired() {
    let device = fake_device("expiry", 2_000_000);
    let policy = PmQosPolicy {
        max_hold: Duration::ZERO,
        ..PmQosPolicy::default()
    };
    let held = prepare(&device, 50, &policy, &ac())
        .unwrap()
        .commit(&ac())
        .unwrap();
    assert!(held.expired(Instant::now()));
}
