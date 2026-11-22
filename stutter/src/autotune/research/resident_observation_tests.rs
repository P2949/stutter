use serde_json::json;

use super::*;

fn processes() -> Vec<AttemptProcess> {
    (0..2)
        .map(|slot| AttemptProcess {
            slot,
            pid: stutter_core::ids::Pid::new(100 + slot),
            start_ticks: 50 + u64::from(slot),
            uid: 1000,
            started_unix: 1_000,
            completed_unix: 1_100,
        })
        .collect()
}

fn entry(slot: u32) -> serde_json::Value {
    json!({
        "slot": slot, "pid": 100 + slot, "start_ticks": 50 + slot, "uid": 1000,
        "window_start_unix": 1_005, "window_end_unix": 1_095,
        "renderer_bound": true, "cgroup": "0::/app.slice/game", "cpu_samples": 60,
        "gpu_samples": 60, "psi_samples": 60, "lost_samples": 0, "owners_identified": true,
        "runnable_events": 5000, "runnable_lost": 0, "runnable_claim": false,
    })
}

fn record(edit: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
    let mut value = json!({ "schema": 2, "attempts": [entry(0), entry(1)] });
    edit(&mut value);
    serde_json::to_vec(&value).unwrap()
}

#[test]
fn an_observation_of_every_completed_attempts_own_process_passes_and_each_gap_is_named() {
    assert_eq!(verify(&record(|_| {}), &processes()), Ok(()));
    let refused = |edit: &dyn Fn(&mut serde_json::Value), why: &'static str| {
        assert_eq!(verify(&record(edit), &processes()), Err(why));
    };
    refused(
        &|v| {
            v["attempts"].as_array_mut().unwrap().pop();
        },
        "ResidentObservationMissingForAnAttempt",
    );
    refused(
        &|v| v["attempts"][1] = v["attempts"][0].clone(),
        "ResidentObservationMissingForAnAttempt",
    );
    refused(
        &|v| v["attempts"][0]["pid"] = 999.into(),
        "ResidentObservationForAnotherProcess",
    );
    refused(
        &|v| v["attempts"][0]["window_start_unix"] = 900.into(),
        "ResidentObservationOutsideTheAttempt",
    );
    refused(
        &|v| v["attempts"][1]["window_end_unix"] = 2_000.into(),
        "ResidentObservationOutsideTheAttempt",
    );
    refused(
        &|v| v["attempts"][0]["renderer_bound"] = false.into(),
        "ResidentObservationIncomplete",
    );
    refused(
        &|v| v["attempts"][1]["owners_identified"] = false.into(),
        "ResidentObservationIncomplete",
    );
    refused(
        &|v| v["attempts"][0]["lost_samples"] = 1.into(),
        "ResidentObservationSourceLossy",
    );
    refused(
        &|v| v["attempts"][0]["psi_samples"] = 3.into(),
        "ResidentObservationTooSparse",
    );
    refused(
        &|v| v["attempts"].as_array_mut().unwrap().push(entry(5)),
        "ResidentObservationForNoCompletedAttempt",
    );
    refused(&|v| v["schema"] = 1.into(), "ResidentObservationInvalid");
    assert_eq!(
        verify(b"nope", &processes()),
        Err("ResidentObservationInvalid")
    );
}

#[test]
fn the_runnable_event_floor_binds_only_an_attempt_that_claims_a_runnable_measurement() {
    // A quiet, valid benchmark with few runnable events is fully observed and passes.
    let quiet = record(|v| {
        for slot in 0..2 {
            v["attempts"][slot]["runnable_events"] = 12.into();
        }
    });
    assert_eq!(verify(&quiet, &processes()), Ok(()));
    // The same attempt used for a runnable-latency claim must meet the floor and be lossless.
    let claimed = |events: u64, lost: u64| {
        record(|v| {
            v["attempts"][0]["runnable_claim"] = true.into();
            v["attempts"][0]["runnable_events"] = events.into();
            v["attempts"][0]["runnable_lost"] = lost.into();
        })
    };
    assert_eq!(
        verify(&claimed(999, 0), &processes()),
        Err("ResidentObservationRunnableFloor")
    );
    assert_eq!(
        verify(&claimed(5000, 2), &processes()),
        Err("ResidentObservationRunnableFloor")
    );
    assert_eq!(verify(&claimed(1000, 0), &processes()), Ok(()));
}
