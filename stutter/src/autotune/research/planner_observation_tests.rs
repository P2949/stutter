use serde_json::{Value, json};

use super::*;

fn processes() -> Vec<AttemptProcess> {
    (0..8)
        .map(|slot| AttemptProcess {
            slot,
            pid: stutter_core::ids::Pid::new(100 + slot),
            start_ticks: 50 + u64::from(slot),
            uid: 1000,
            started_unix: 1_000,
            completed_unix: 2_200,
        })
        .collect()
}

fn entry(slot: u32) -> Value {
    json!({
        "slot": slot, "process": { "pid": 100 + slot, "start_ticks": 50 + slot, "uid": 1000 },
        "start_unix": 1_010, "end_unix": 2_190, "evaluations": 600, "candidates": 0, "lost": 0,
        "source_quality": "direct",
    })
}

fn record(edit: impl FnOnce(&mut Value)) -> Vec<u8> {
    let mut value = json!({
        "schema": 1, "action": "CpuAffinityProfile",
        "attempts": (0..8).map(entry).collect::<Vec<_>>(),
    });
    edit(&mut value);
    serde_json::to_vec(&value).unwrap()
}

fn check(bytes: &[u8]) -> Result<(), &'static str> {
    verify(bytes, ResearchAction::CpuAffinityProfile, &processes())
}

#[test]
fn an_abstention_stands_only_on_what_the_planner_saw_in_every_attempt() {
    assert_eq!(check(&record(|_| {})), Ok(()));
    let refused = |edit: &dyn Fn(&mut Value), why: &'static str| {
        assert_eq!(check(&record(edit)), Err(why));
    };
    refused(
        &|v| v["action"] = "GpuClockPolicy".into(),
        "PlannerObservationForAnotherAction",
    );
    refused(
        &|v| v["schema"] = 2.into(),
        "PlannerObservationForAnotherAction",
    );
    // An attempt unobserved, observed twice, or an attempt that never ran.
    refused(
        &|v| {
            v["attempts"].as_array_mut().unwrap().pop();
        },
        "PlannerObservationNotEveryAttempt",
    );
    refused(
        &|v| v["attempts"][7] = entry(0),
        "PlannerObservationNotEveryAttempt",
    );
    refused(
        &|v| v["attempts"][3]["slot"] = 9.into(),
        "PlannerObservationNotEveryAttempt",
    );
    refused(
        &|v| v["attempts"][2]["process"]["pid"] = 999.into(),
        "PlannerObservationForAnotherProcess",
    );
    refused(
        &|v| v["attempts"][2]["start_unix"] = 900.into(),
        "PlannerObservationOutsideTheAttempt",
    );
    refused(
        &|v| v["attempts"][2]["end_unix"] = 3_000.into(),
        "PlannerObservationOutsideTheAttempt",
    );
    refused(
        &|v| v["attempts"][1]["lost"] = 1.into(),
        "PlannerObservationSourceNotQualified",
    );
    refused(
        &|v| v["attempts"][1]["source_quality"] = "derived".into(),
        "PlannerObservationSourceNotQualified",
    );
    refused(
        &|v| v["attempts"][4]["evaluations"] = 3.into(),
        "PlannerObservationTooFewEvaluations",
    );
    // A planner that did propose the action was not abstaining.
    refused(
        &|v| v["attempts"][5]["candidates"] = 1.into(),
        "PlannerHadACandidate",
    );
    assert_eq!(check(b"{"), Err("PlannerObservationUnreadable"));
    // No completed attempt at all is nothing observed.
    assert_eq!(
        verify(&record(|_| {}), ResearchAction::CpuAffinityProfile, &[]),
        Err("PlannerObservationNotEveryAttempt")
    );
}

/// A pass the planner ran but that denied the authorized action (policy, capability, workload) is a refusal, not a
/// finding that no action was indicated: the record that carries one cannot support a justified abstention.
#[test]
fn an_attempt_in_which_the_planner_denied_the_action_is_not_a_justified_abstention() {
    let denied = record(|v| v["attempts"][3]["denied"] = 2.into());
    assert_eq!(check(&denied), Err("PlannerDeniedTheAction"));
    // A record without the field (written before it existed) reads as none denied.
    assert_eq!(check(&record(|_| {})), Ok(()));
}

/// The sealed observation is only what the attempts' own raw windows counted: an entry that differs from its window in
/// any counter, the quality, the interval or that has no window for its attempt is refused.
#[test]
fn a_planner_observation_that_is_not_what_its_raw_windows_counted_is_refused() {
    let window = |slot: u32| {
        json!({
            "attempt": slot, "pid": 100 + slot, "start_ticks": 50 + slot, "uid": 1000,
            "started_unix": 1_010, "ended_unix": 2_190, "lost_events": 0, "invalid_intervals": 0,
            "cpu_samples": 600, "evaluations": 600, "denied_evaluations": 0,
            "affinity_candidates": 0, "cgroup_candidates": 0,
        })
    };
    let windows = || Value::Array((0..8).map(window).collect());
    let matches = |windows: &Value| {
        matches_windows(
            &record(|_| {}),
            ResearchAction::CpuAffinityProfile,
            windows,
            &processes(),
        )
    };
    assert_eq!(matches(&windows()), Ok(()));
    let altered = |edit: &dyn Fn(&mut Value)| {
        let mut all = windows();
        edit(&mut all[3]);
        matches(&all)
    };
    let why = Err("PlannerObservationNotFromTheWindows");
    // One pass fewer, a candidate the record says never came, a denial, a lost event, no scheduler samples, or another
    // interval each contradict the sealed entry.
    assert_eq!(altered(&|w| w["evaluations"] = 599.into()), why);
    assert_eq!(altered(&|w| w["affinity_candidates"] = 1.into()), why);
    assert_eq!(altered(&|w| w["denied_evaluations"] = 1.into()), why);
    assert_eq!(altered(&|w| w["lost_events"] = 1.into()), why);
    assert_eq!(altered(&|w| w["cpu_samples"] = 0.into()), why);
    assert_eq!(altered(&|w| w["started_unix"] = 1_100.into()), why);
    // An attempt with no window of its own.
    let mut fewer = windows();
    fewer.as_array_mut().unwrap().pop();
    assert_eq!(matches(&fewer), why);
    // Another attempt's window does not stand in for it, even for the very same process.
    let mut moved = windows();
    moved[3]["attempt"] = 9.into();
    assert_eq!(matches(&moved), why);
}
