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
                    long_session: None,
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
            long_session: None,
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
        context: None,
        loaded_runtime: Some(matched_runtime()),
        runtime_binding: Some(super::super::loaded_runtime::RuntimeBinding::Matched),
        files: vec![NativeBenchmarkFile {
            path: "result/summary.json".into(),
            size_bytes: 10,
            sha256: "d".repeat(64),
        }],
    };
    let mut evidence = AttemptEvidenceV1 {
        long_session: None,
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

#[test]
fn inconsistent_native_context_withholds_the_numeric_comparison() {
    // Missing native completions are unproven, never stable.
    let (_, schedule) = study(6);
    let (_, evidence) = run(&schedule, |_| Some(60.0));
    assert_eq!(context_state(&evidence), ContextState::Unproven);
    assert_eq!(context_state(&BTreeMap::new()), ContextState::Unproven);
}

#[test]
fn native_completions_decide_consistent_or_drifted_context() {
    use super::super::cyberpunk::{
        CyberpunkBenchmarkCompletion, CyberpunkProcessIdentity, CyberpunkScenario,
        NativeBenchmarkFile, NativeBenchmarkMetrics,
    };
    let (intent, schedule) = study(4);
    let make = |slot: &AttemptSlot| {
        let tool = intent.tools.iter().find(|t| t.source == slot.arm).unwrap();
        AttemptEvidenceV1 {
            long_session: None,
            schema_version: 1,
            slot_index: slot.index,
            average_fps: 60.0,
            native_completion: Some(Box::new(CyberpunkBenchmarkCompletion {
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
                context: Some(super::super::native_context::NativeContext {
                    game_version: Some("2.5".into()),
                    preset_name: Some("RayTracingUltra".into()),
                    gpu_name: Some("GPU".into()),
                    upscaling_type: Some(2),
                    ray_tracing_enabled: Some(true),
                    frame_generation_type: Some(0),
                    frame_count: 100,
                    frame_stats: None,
                }),
                loaded_runtime: Some(matched_runtime()),
                runtime_binding: Some(super::super::loaded_runtime::RuntimeBinding::Matched),
                files: vec![NativeBenchmarkFile {
                    path: "result/summary.json".into(),
                    size_bytes: 10,
                    sha256: "d".repeat(64),
                }],
            })),
        }
    };
    let mut evidence: BTreeMap<u32, _> = schedule.iter().map(|s| (s.index, make(s))).collect();
    assert_eq!(context_state(&evidence), ContextState::Consistent);
    let mut feature = evidence.clone();
    feature
        .get_mut(&3)
        .unwrap()
        .native_completion
        .as_mut()
        .unwrap()
        .context
        .as_mut()
        .unwrap()
        .frame_generation_type = Some(1);
    let factor = super::super::TreatmentSpec::Factor(super::super::ResearchFactor::FrameGeneration);
    assert!(evidence_context_consistent(&feature, Some(&factor)));
    assert!(!evidence_context_consistent(&feature, None));
    feature
        .get_mut(&3)
        .unwrap()
        .native_completion
        .as_mut()
        .unwrap()
        .context
        .as_mut()
        .unwrap()
        .ray_tracing_enabled = Some(false);
    assert!(!evidence_context_consistent(&feature, Some(&factor)));
    evidence
        .get_mut(&3)
        .unwrap()
        .native_completion
        .as_mut()
        .unwrap()
        .settings_sha256 = "e".repeat(64);
    assert_eq!(context_state(&evidence), ContextState::Drifted);
    evidence.get_mut(&3).unwrap().native_completion = None;
    // One unproven attempt keeps an otherwise consistent set from being called stable.
    assert_eq!(context_state(&evidence), ContextState::Unproven);
}

#[test]
fn a_per_comparison_subset_without_native_completions_is_unproven() {
    let (intent, schedule) = study(4);
    let (_, evidence) = run(&schedule, |_| Some(60.0));
    let subset: BTreeMap<u32, _> = evidence
        .iter()
        .filter(|(i, _)| schedule[**i as usize].arm == intent.tools[0].source)
        .map(|(i, e)| (*i, e.clone()))
        .collect();
    assert_eq!(context_state(&subset), ContextState::Unproven);
}

#[test]
fn an_unrelated_arm_failing_never_discards_a_complete_control_candidate_pair() {
    let mut intent = intent();
    intent.tools.push(ResearchTool {
        source: ProtonSource::ValveStable,
        identity_sha256: "b".repeat(64),
    });
    intent.tools.push(ResearchTool {
        source: ProtonSource::CachyOs,
        identity_sha256: "c".repeat(64),
    });
    assert_eq!(
        intent.tools.len(),
        3,
        "the fixture must start with one tool"
    );
    intent.max_attempts = 18;
    let schedule = build_schedule(&intent).unwrap();
    let blocks = schedule.len() / 3;
    let (records, evidence) = run(&schedule, |slot| {
        // The third arm crashes in the first three blocks; the other two always complete.
        if slot.arm == ProtonSource::CachyOs && (slot.index as usize) / 3 < 3 {
            return None;
        }
        Some(if slot.arm == ProtonSource::ValveStable {
            60.0
        } else {
            66.0 + f64::from(slot.index % 3) * 0.1
        })
    });
    let report = build_report(&intent, &records, &evidence).unwrap();
    let pairs = |source: ProtonSource| {
        report
            .arms
            .iter()
            .find(|arm| arm.source == source)
            .map(|arm| arm.pairs)
    };
    assert_eq!(pairs(ProtonSource::Ge), Some(blocks));
    assert_eq!(pairs(ProtonSource::CachyOs), Some(blocks - 3));
}

/// A receipt of a complete Direct3D 12 stack, as the host records it for a matched arm.
fn matched_runtime() -> super::super::loaded_runtime::LoadedRuntime {
    super::super::loaded_runtime::LoadedRuntime {
        mapped_file_count: 3,
        wine_loaded: true,
        vkd3d_loaded: true,
        dxvk_loaded: true,
        proton_roots: vec!["/nix/store/aaa-proton/files".into()],
        runtime_paths_sha256: "a".repeat(64),
    }
}

#[test]
fn a_runtime_binding_without_its_receipt_or_over_a_partial_stack_is_rejected() {
    use super::super::loaded_runtime::RuntimeBinding;
    let (intent, schedule) = study(4);
    let slot = &schedule[0];
    let build = |receipt: Option<super::super::loaded_runtime::LoadedRuntime>,
                 binding: Option<RuntimeBinding>| {
        let mut item = valid_native_evidence(&intent, slot);
        let native = item.native_completion.as_mut().unwrap();
        native.loaded_runtime = receipt;
        native.runtime_binding = binding;
        item.validate_for(slot, &intent).is_ok()
    };
    let matched = Some(RuntimeBinding::Matched);
    assert!(build(Some(matched_runtime()), matched));
    assert!(
        build(None, None),
        "no runtime captured is allowed (unproven)"
    );
    assert!(
        !build(None, matched),
        "a binding without the receipt it came from"
    );
    assert!(
        !build(Some(matched_runtime()), None),
        "a receipt without a binding"
    );
    let mut partial = matched_runtime();
    partial.vkd3d_loaded = false;
    assert!(
        !build(Some(partial), matched),
        "Matched over a partial D3D12 stack"
    );
    let mut rootless = matched_runtime();
    rootless.proton_roots.clear();
    assert!(!build(Some(rootless), matched));
    assert!(build(
        Some(matched_runtime()),
        Some(RuntimeBinding::Unknown)
    ));
}

fn valid_native_evidence(intent: &ResearchIntentV1, slot: &AttemptSlot) -> AttemptEvidenceV1 {
    use super::super::cyberpunk::{
        CyberpunkBenchmarkCompletion, CyberpunkProcessIdentity, CyberpunkScenario,
        NativeBenchmarkFile, NativeBenchmarkMetrics,
    };
    let tool = intent
        .tools
        .iter()
        .find(|tool| tool.source == slot.arm)
        .unwrap();
    AttemptEvidenceV1 {
        long_session: None,
        schema_version: 1,
        slot_index: slot.index,
        average_fps: 60.0,
        native_completion: Some(Box::new(CyberpunkBenchmarkCompletion {
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
            context: None,
            loaded_runtime: None,
            runtime_binding: None,
            files: vec![NativeBenchmarkFile {
                path: "result/summary.json".into(),
                size_bytes: 10,
                sha256: "d".repeat(64),
            }],
        })),
    }
}

#[test]
fn native_file_sizes_that_overflow_are_an_error_not_a_saturated_pass() {
    use super::super::cyberpunk::NativeBenchmarkFile;
    let (intent, schedule) = study(4);
    let mut item = valid_native_evidence(&intent, &schedule[0]);
    let file = |size| NativeBenchmarkFile {
        path: "result/f".into(),
        size_bytes: size,
        sha256: "d".repeat(64),
    };
    item.native_completion.as_mut().unwrap().files = vec![file(u64::MAX), file(2)];
    assert!(super::super::report_store::evidence_cost(&item).is_err());
    item.native_completion.as_mut().unwrap().files = vec![file(10), file(20)];
    assert!(super::super::report_store::evidence_cost(&item).unwrap() >= 30);
}

#[test]
fn a_confirmation_design_uses_its_pilots_frozen_thresholds_not_the_provisional_defaults() {
    use super::super::pilot_rule::freeze;
    let (_, schedule) = study(4);
    let rule = freeze(&[70.0, 73.0, 67.0]).unwrap();
    let frozen = design(
        ProtonSource::Ge,
        &schedule,
        4,
        0.95,
        1,
        Some(&rule),
        NativeMetric::AverageFps,
    );
    assert_eq!(
        frozen.min_effect_percent,
        f64::from(rule.min_effect_bp) / 100.0
    );
    assert_eq!(
        frozen.equivalence_margin_percent,
        f64::from(rule.equivalence_margin_bp) / 100.0
    );
    let default = design(
        ProtonSource::Ge,
        &schedule,
        4,
        0.95,
        1,
        None,
        NativeMetric::AverageFps,
    );
    assert_eq!(default.min_effect_percent, MIN_EFFECT_PERCENT);
    assert_ne!(frozen.min_effect_percent, default.min_effect_percent);
}

#[test]
fn zero_warmup_and_washout_rest_on_whole_launches_in_complete_order_balanced_blocks() {
    let (intent, schedule) = study(6);
    let design = design(
        ProtonSource::ValveStable,
        &schedule,
        6,
        0.95,
        1,
        None,
        NativeMetric::AverageFps,
    );
    assert_eq!((design.warmup_seconds, design.washout_seconds), (0, 0));
    // Every block holds every arm exactly once, and no arm always goes first: carry-over between
    // launches is balanced by the schedule, which is what stands in for discarded time.
    let arms = intent.tools.len();
    let mut firsts = std::collections::BTreeMap::new();
    for block in schedule.chunks(arms) {
        let mut seen: Vec<_> = block.iter().map(|slot| slot.arm).collect();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), arms, "a complete block");
        *firsts.entry(block[0].arm).or_insert(0) += 1;
    }
    assert!(
        firsts.values().all(|count| *count < 6),
        "no arm leads every block: {firsts:?}"
    );
}

mod within_source {
    use super::*;
    use crate::autotune::research::{
        ResearchFactor, Treatment, TreatmentSpec, treatment::treatments_of,
    };

    fn treated() -> (ResearchIntentV1, Vec<AttemptSlot>, Vec<Treatment>) {
        let mut intent = intent();
        intent.factors = vec![ResearchFactor::NtSync];
        intent.child_study_sha256 = Some("ab".repeat(32));
        intent.treatment = Some(TreatmentSpec::Factor(ResearchFactor::NtSync));
        intent.max_attempts = 8;
        let schedule = build_schedule(&intent).unwrap();
        let treatments = treatments_of(&intent).unwrap().unwrap();
        (intent, schedule, treatments)
    }

    #[test]
    fn one_build_with_the_switch_on_versus_off_is_one_paired_result_that_rebuilds_identically() {
        let (intent, schedule, treatments) = treated();
        let (records, evidence) = run(&schedule, |slot| {
            let on = treatments[slot.index as usize] == Treatment::On;
            Some(if on { 77.0 } else { 70.0 } + f64::from(slot.index) * 0.01)
        });
        let report = build_report(&intent, &records, &evidence).unwrap();
        assert_eq!(report.control, ProtonSource::Ge);
        assert_eq!(report.arms.len(), 1, "one result, not a second Proton");
        assert_eq!(report.arms[0].source, ProtonSource::Ge);
        assert_eq!(report.arms[0].pairs, 4);
        assert_eq!(report.arms[0].numeric_verdict.as_deref(), Some("improved"));
        assert_eq!(report.treatment, intent.treatment);
        // Not provenance-qualified: the fixture sealed no native completion.
        assert_eq!(report.arms[0].status, "unqualified_provenance");
        verify_report(&intent, &records, &evidence, &report).unwrap();
    }

    #[test]
    fn a_block_missing_either_treatment_is_not_a_pair() {
        let (intent, schedule, treatments) = treated();
        // The first block's "on" attempt did not complete.
        let on_first = treatments.iter().position(|t| *t == Treatment::On).unwrap();
        let (records, evidence) = run(&schedule, |slot| {
            if slot.index as usize == on_first {
                return None;
            }
            Some(70.0 + f64::from(slot.index) * 0.01)
        });
        let report = build_report(&intent, &records, &evidence).unwrap();
        assert_eq!(
            report.arms[0].pairs, 3,
            "an incomplete block is dropped whole"
        );
    }

    #[test]
    fn a_plain_report_carries_no_treatment_and_keeps_its_bytes() {
        let (intent, schedule) = study(6);
        let (records, evidence) = run(&schedule, |_| Some(70.0));
        let report = build_report(&intent, &records, &evidence).unwrap();
        assert_eq!(report.treatment, None);
        assert!(
            !serde_json::to_string(&report)
                .unwrap()
                .contains("treatment")
        );
    }
}

#[path = "report_context_tests.rs"]
mod context;
