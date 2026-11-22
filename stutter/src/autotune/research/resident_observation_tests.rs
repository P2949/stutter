use serde_json::json;

use super::*;

fn processes() -> Vec<AttemptProcess> {
    (0..2)
        .map(|slot| AttemptProcess {
            slot,
            pid: stutter_core::ids::Pid::new(100 + slot),
            start_ticks: 50 + u64::from(slot),
            uid: 1000,
            started_unix: 0,
            completed_unix: 1,
        })
        .collect()
}

fn entry(slot: u32) -> serde_json::Value {
    json!({
        "slot": slot, "pid": 100 + slot, "start_ticks": 50 + slot, "uid": 1000,
        "renderer_bound": true, "cgroup": "0::/app.slice/game", "cpu_samples": 60,
        "gpu_samples": 60, "psi_samples": 60, "runnable_events": 5000,
        "owners_identified": true, "quality": "Eligible",
    })
}

fn record(edit: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
    let mut value = json!({ "schema": 1, "attempts": [entry(0), entry(1)] });
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
        &|v| v["attempts"][0]["quality"] = "Stale".into(),
        "ResidentObservationSourceNotQualified",
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
        &|v| v["attempts"][0]["psi_samples"] = 3.into(),
        "ResidentObservationTooSparse",
    );
    refused(
        &|v| v["attempts"][1]["runnable_events"] = 999.into(),
        "ResidentObservationTooSparse",
    );
    refused(
        &|v| v["attempts"].as_array_mut().unwrap().push(entry(5)),
        "ResidentObservationForNoCompletedAttempt",
    );
    assert_eq!(
        verify(b"nope", &processes()),
        Err("ResidentObservationInvalid")
    );
}
