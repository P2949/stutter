use std::time::Duration;

use super::*;
use crate::actions::{
    pm_qos::{PmQosPolicy, PowerContext},
    pm_qos_transaction::prepare,
};

fn ac() -> PowerContext {
    PowerContext {
        on_ac: Some(true),
        throttle_events_recent: Some(0),
    }
}

fn held(name: &str, hold: Duration) -> PmQosHeld {
    let device =
        std::env::temp_dir().join(format!("stutter-pmqos-wd-{name}-{}", std::process::id()));
    std::fs::write(&device, 2_000_000_u32.to_ne_bytes()).unwrap();
    let policy = PmQosPolicy {
        max_hold: hold,
        ..PmQosPolicy::default()
    };
    prepare(&device, 50, &policy, &ac())
        .unwrap()
        .commit(&ac())
        .unwrap()
}

#[test]
fn the_deadline_releases_a_lease_its_owner_never_returns() {
    let watchdog = enforce_deadline(held("deadline", Duration::from_millis(50))).unwrap();
    let started = Instant::now();
    while !watchdog.finished() {
        assert!(started.elapsed() < Duration::from_secs(5), "no release");
        std::thread::sleep(Duration::from_millis(5));
    }
    let result = watchdog.release();
    assert!(result.ended_by_deadline);
}

#[test]
fn an_early_release_does_not_wait_for_the_deadline() {
    let watchdog = enforce_deadline(held("early", Duration::from_secs(60))).unwrap();
    let started = Instant::now();
    let result = watchdog.release();
    assert!(!result.ended_by_deadline);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn an_owner_limit_can_only_shorten_the_policy_limit() {
    let watchdog = enforce_deadline_within(
        held("shorter", Duration::from_millis(80)),
        Some(Duration::from_secs(600)),
    )
    .unwrap();
    let started = Instant::now();
    while !watchdog.finished() {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "owner limit extended the hold"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(watchdog.release().ended_by_deadline);
}

#[test]
fn dropping_the_watchdog_releases_the_lease() {
    let watchdog = enforce_deadline(held("dropped", Duration::from_secs(60))).unwrap();
    let started = Instant::now();
    drop(watchdog);
    assert!(started.elapsed() < Duration::from_secs(5));
}
