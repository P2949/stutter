use super::*;

/// The measurement the study's objective needs, built by the evidence collectors from a tail the action shortened
/// in every pair: with it sealed beside the receipts the same shared judgement that refused the study without it now
/// qualifies it, and the replay (which recomputes the tail from the populations) agrees with the live outcome.
#[test]
fn a_long_action_study_with_its_sealed_latency_measurement_qualifies_and_replays() {
    use std::os::unix::fs::PermissionsExt;

    use crate::autotune::research::{
        action_latency,
        child_verify::AttemptProcess,
        evidence_producers::{
            LatencyWindowBuilder, WindowGuards, action_latency_document, latency_pair,
        },
        treatment::{Treatment, treatments_of},
    };
    let (root, mut runner, study, gate, parent) = build(&unit().definition_sha256);
    assert_eq!(runner.poll(&study, 100).unwrap(), None);
    let path = root.path().join("studies").join(&study.study_sha256[..16]);
    let intent: ResearchIntentV1 =
        serde_json::from_slice(&std::fs::read(path.join("intent.json")).unwrap()).unwrap();
    let schedule = treatments_of(&intent).unwrap().unwrap();
    let process = |slot: u32| AttemptProcess {
        slot,
        pid: attempt_process(slot).pid,
        start_ticks: attempt_process(slot).start_ticks,
        uid: attempt_process(slot).uid,
        started_unix: 5_000,
        completed_unix: 6_200,
    };
    let window = |tail_ns: u64| {
        let mut builder = LatencyWindowBuilder::new(5_010);
        for _ in 0..1_900 {
            builder.event(500_000);
        }
        for _ in 0..100 {
            builder.event(tail_ns);
        }
        builder.finish(
            6_190,
            WindowGuards {
                foreground_p99_ms: 12.0,
                foreground_samples: 1_000,
                foreground_over_5ms: 10,
                thermal_degraded: false,
                thermal_throttle_count: 0,
                cpu_power_limited: false,
                gpu_power_limited: false,
            },
        )
    };
    let record = |off_tail: u64, on_tail: u64| {
        action_latency_document(
            (0..schedule.len() as u32 / 2)
                .map(|block| {
                    let (a, b) = (block * 2, block * 2 + 1);
                    let (off, on) = if schedule[a as usize] == Treatment::Off {
                        (a, b)
                    } else {
                        (b, a)
                    };
                    latency_pair(
                        (&process(off), window(off_tail)),
                        (&process(on), window(on_tail)),
                    )
                })
                .collect(),
        )
    };
    let file = path.join(action_latency::FILE);
    std::fs::write(&file, record(20_000_000, 1_500_000)).unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    let outcome = run_acting(&mut runner, &study, &gate);
    assert!(
        outcome.action_metric_valid,
        "the sealed measurement is a valid one"
    );
    let run = judge(&study, &outcome);
    assert_eq!(
        (run.result, run.reason),
        (StudyResult::Qualified, ""),
        "{outcome:?}"
    );
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path.join("report.json")).unwrap()).unwrap();
    let disposition = crate::autotune::research::child_ledger::Disposition {
        study_sha256: study.study_sha256.clone(),
        profile: study.profile,
        stage: study.stage,
        result: Some(StudyResult::Qualified),
        reason: String::new(),
    };
    let dir = std::fs::File::open(&path).unwrap();
    let replayed = crate::autotune::research::child_judgement::verify_qualification(
        &dir,
        crate::syscall::geteuid(),
        &parent,
        &disposition,
        &report,
        true,
        true,
    );
    assert!(replayed.is_ok(), "{replayed:?}");
    // Replay: the same function over the same bytes, and a worse tail is a measurement that is still valid but
    // decides Revert, never Keep.
    let bytes = std::fs::read(&file).unwrap();
    let processes: Vec<_> = (0..schedule.len() as u32).map(process).collect();
    let result = action_latency::verify(&bytes, &schedule, &processes).unwrap();
    assert_eq!(
        (result.pairs, result.decision),
        (4, action_latency::Decision::Keep)
    );
    let worse =
        action_latency::verify(&record(1_500_000, 20_000_000), &schedule, &processes).unwrap();
    assert_eq!(worse.decision, action_latency::Decision::Revert);
    // A record that names another process cannot stand for these attempts.
    let mut stranger = process(0);
    stranger.pid = stutter_core::ids::Pid::new(1);
    let mut moved = processes.clone();
    moved[0] = stranger;
    assert!(action_latency::verify(&bytes, &schedule, &moved).is_err());
}
