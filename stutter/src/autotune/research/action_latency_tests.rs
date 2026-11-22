use serde_json::json;

use super::*;

fn processes(slots: u32) -> Vec<AttemptProcess> {
    (0..slots)
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

fn schedule() -> Vec<Treatment> {
    [Treatment::Off, Treatment::On].repeat(3)
}

fn pair(block: u32, off: f64, on: f64) -> serde_json::Value {
    let (a, b) = (block * 2, block * 2 + 1);
    json!({
        "off_slot": a, "on_slot": b,
        "off_process": { "pid": 100 + a, "start_ticks": 50 + a, "uid": 1000 },
        "on_process": { "pid": 100 + b, "start_ticks": 50 + b, "uid": 1000 },
        "off_events": 2000, "on_events": 2000, "off_p95_ms": off, "on_p95_ms": on,
    })
}

fn record(edit: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
    let mut value = json!({
        "schema": 1, "metric": "GameRunnableLatency", "source_quality": "Eligible",
        "guardrails_held": true,
        "pairs": [pair(0, 9.0, 5.0), pair(1, 8.0, 5.5), pair(2, 8.5, 8.0)],
    });
    edit(&mut value);
    serde_json::to_vec(&value).unwrap()
}

fn check(bytes: &[u8]) -> Result<LatencyResult, &'static str> {
    verify(bytes, &schedule(), &processes(6))
}

#[test]
fn a_complete_paired_qualified_record_gives_its_result_and_null_results_are_still_valid() {
    assert_eq!(
        check(&record(|_| {})),
        Ok(LatencyResult {
            pairs: 3,
            improved: 3
        })
    );
    // No improvement is a valid measurement, not a failure to measure.
    let flat = record(|v| {
        v["pairs"] = json!([pair(0, 5.0, 5.0), pair(1, 5.0, 5.2), pair(2, 5.0, 5.1)]);
    });
    assert_eq!(
        check(&flat),
        Ok(LatencyResult {
            pairs: 3,
            improved: 0
        })
    );
}

#[test]
fn every_way_a_record_can_fail_to_be_a_measurement_has_its_own_refusal() {
    let refused = |edit: &dyn Fn(&mut serde_json::Value), why: &'static str| {
        assert_eq!(check(&record(edit)), Err(why));
    };
    refused(
        &|v| v["metric"] = "AverageFps".into(),
        "ActionLatencyWrongMetric",
    );
    refused(
        &|v| v["source_quality"] = "Lossy".into(),
        "ActionLatencySourceNotQualified",
    );
    refused(
        &|v| v["guardrails_held"] = false.into(),
        "ActionLatencyGuardrailBroken",
    );
    refused(
        &|v| {
            v["pairs"].as_array_mut().unwrap().pop();
        },
        "ActionLatencyTooFewPairs",
    );
    refused(
        &|v| v["pairs"][0]["off_events"] = 999.into(),
        "ActionLatencyTooFewEvents",
    );
    refused(
        &|v| v["pairs"][1]["on_p95_ms"] = 0.0.into(),
        "ActionLatencyNotFinite",
    );
    // A pair that is not one block's Off then On, or that reuses an attempt.
    refused(
        &|v| v["pairs"][1]["off_slot"] = 3.into(),
        "ActionLatencyPairNotInTheSchedule",
    );
    refused(
        &|v| v["pairs"][2] = v["pairs"][0].clone(),
        "ActionLatencyPairNotInTheSchedule",
    );
    // Another game process than the attempt recorded.
    refused(
        &|v| v["pairs"][0]["on_process"]["pid"] = 999.into(),
        "ActionLatencyForAnotherProcess",
    );
    assert_eq!(check(b"{"), Err("ActionLatencyUnreadable"));
}
