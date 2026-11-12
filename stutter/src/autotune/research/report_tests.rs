use super::*;
use crate::autotune::research::{ResearchTool, tests::intent};

fn study(blocks: u32) -> (ResearchIntentV1, Vec<AttemptSlot>) {
    let mut intent = intent();
    intent.tools.push(ResearchTool {
        source: ProtonSource::ValveStable,
        identity_sha256: "b".repeat(64),
    });
    intent.max_attempts = blocks * 2;
    let schedule = build_schedule(&intent).unwrap();
    (intent, schedule)
}

fn run(
    schedule: &[AttemptSlot],
    fps: impl Fn(&AttemptSlot) -> Option<f64>,
) -> (Vec<AttemptRecord>, BTreeMap<u32, AttemptEvidenceV1>) {
    let mut records = Vec::new();
    let mut evidence = BTreeMap::new();
    for slot in schedule {
        let outcome = match fps(slot) {
            Some(average_fps) => {
                let item = AttemptEvidenceV1 {
                    schema_version: 1,
                    slot_index: slot.index,
                    average_fps,
                    native_completion: None,
                };
                let digest = item.digest().unwrap();
                evidence.insert(slot.index, item);
                AttemptOutcome::Completed {
                    evidence_sha256: digest,
                }
            }
            None => AttemptOutcome::Failed {
                code: "GameCrashed".into(),
            },
        };
        records.push(AttemptRecord {
            slot: slot.clone(),
            outcome,
        });
    }
    (records, evidence)
}

#[test]
fn numeric_gain_is_reconstructed_but_not_qualified() {
    let (intent, schedule) = study(6);
    let (records, evidence) = run(&schedule, |slot| {
        let wobble = f64::from(slot.index % 3) * 0.2;
        Some(if slot.arm == ProtonSource::ValveStable {
            60.0 + wobble
        } else {
            66.0 + wobble
        })
    });
    let report = build_report(&intent, &records, &evidence).unwrap();
    assert_eq!(report.control, ProtonSource::ValveStable);
    assert_eq!(report.schema_version, 2);
    assert_eq!(report.arms.len(), 1);
    assert_eq!(report.arms[0].status, "unqualified_provenance");
    assert_eq!(report.arms[0].numeric_verdict.as_deref(), Some("improved"));
    assert_eq!(report.arms[0].pairs, 6);
    let mean = report.arms[0].mean_effect_percent.unwrap();
    assert!(
        (9.9..=10.1).contains(&mean),
        "expected roughly 10 percent, got {mean}"
    );
    assert!(report.arms[0].ci_low_percent.unwrap() > 9.0);
    assert!(report.arms[0].ci_high_percent.unwrap() < 11.0);
    verify_report(&intent, &records, &evidence, &report).unwrap();
    let mut forged = report.clone();
    forged.arms[0].status = "regressed".into();
    assert!(verify_report(&intent, &records, &evidence, &forged).is_err());
    let mut old_schema = report.clone();
    old_schema.schema_version = 1;
    assert!(verify_report(&intent, &records, &evidence, &old_schema).is_err());
}

#[test]
fn crashed_blocks_are_dropped_and_too_few_pairs_are_refused_not_guessed() {
    let (intent, schedule) = study(6);
    // Candidate crashes in three of six blocks, leaving fewer than the fixed N pairs.
    let (records, evidence) = run(&schedule, |slot| {
        if slot.arm != ProtonSource::ValveStable && (slot.index / 2) % 2 == 0 {
            None
        } else {
            Some(60.0)
        }
    });
    let report = build_report(&intent, &records, &evidence).unwrap();
    assert_eq!(report.arms[0].pairs, 3);
    assert_eq!(report.arms[0].status, "refused_TooFewPairs");
    assert_eq!(report.attempts_completed, 9);
}

#[test]
fn tampered_evidence_and_schedule_drift_are_rejected() {
    let (intent, schedule) = study(6);
    let (records, mut evidence) = run(&schedule, |_| Some(60.0));
    evidence.get_mut(&0).unwrap().average_fps = 99.0;
    assert!(build_report(&intent, &records, &evidence).is_err());
    let (mut records, evidence) = run(&schedule, |_| Some(60.0));
    records.swap(0, 1);
    assert!(build_report(&intent, &records, &evidence).is_err());
    let (records, mut evidence) = run(&schedule, |_| None);
    evidence.insert(
        0,
        AttemptEvidenceV1 {
            schema_version: 1,
            slot_index: 0,
            average_fps: 60.0,
            native_completion: None,
        },
    );
    assert!(build_report(&intent, &records, &evidence).is_err());
}

#[test]
fn native_completion_must_match_the_frozen_attempt_before_sealing() {
    use super::super::cyberpunk::{
        CyberpunkBenchmarkCompletion, CyberpunkProcessIdentity, CyberpunkScenario,
        NativeBenchmarkFile, NativeBenchmarkMetrics,
    };
    let (intent, schedule) = study(4);
    let slot = &schedule[0];
    let tool = intent
        .tools
        .iter()
        .find(|tool| tool.source == slot.arm)
        .unwrap();
    let native = CyberpunkBenchmarkCompletion {
        schema_version: 1,
        app_id: intent.app_id,
        build_id: intent.build_id,
        scene: CyberpunkScenario::BuiltinBenchmark,
        settings_sha256: "c".repeat(64),
        proton_source: slot.arm,
        proton_identity_sha256: tool.identity_sha256.clone(),
        process: CyberpunkProcessIdentity {
            pid: stutter_core::ids::Pid::new(1234),
            start_ticks: 1,
            uid: intent.uid,
            app_id: intent.app_id,
            build_id: intent.build_id,
            executable_sha256: intent.game_executable_sha256.clone(),
        },
        started_unix_nanos: 1,
        completed_unix_nanos: 2,
        result_directory: "result".into(),
        metrics: NativeBenchmarkMetrics {
            average_fps: 60.0,
            minimum_fps: None,
            maximum_fps: None,
        },
        files: vec![NativeBenchmarkFile {
            path: "result/summary.json".into(),
            size_bytes: 10,
            sha256: "d".repeat(64),
        }],
    };
    let mut evidence = AttemptEvidenceV1 {
        schema_version: 1,
        slot_index: slot.index,
        average_fps: 60.0,
        native_completion: Some(Box::new(native)),
    };
    assert!(evidence.validate_for(slot, &intent).is_ok());
    evidence
        .native_completion
        .as_mut()
        .unwrap()
        .proton_identity_sha256 = "e".repeat(64);
    assert!(evidence.validate_for(slot, &intent).is_err());
}
