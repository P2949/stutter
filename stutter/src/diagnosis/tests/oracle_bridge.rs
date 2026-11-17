//! The oracle producer applied to the diagnosis engine's own output: real `Diagnosis` values from the
//! engine's fixtures, mapped to the oracle's causes and scored against ground truth. This shows the
//! mapping works on what the engine really emits, not only on hand-built structs. It is a fixture
//! exercise of the pipeline and never stands for the physical cases PLAN asks for.

use super::*;
use crate::autotune::research::{
    oracle::{CaseOutcome, Cause, GroundTruth, score_case},
    oracle_cases::from_controller,
};

fn truth(id: &str, injected: Option<Cause>) -> GroundTruth {
    GroundTruth {
        case_id: id.into(),
        injected,
    }
}

#[test]
fn a_real_scheduler_delay_diagnosis_is_scored_as_runnable_delay_and_may_propose_the_action() {
    let cluster = spike_cluster(vec![spike_point(
        456,
        TaskClass::Game,
        "RenderThread",
        8_000_000,
    )]);
    let engine = diagnose_cluster(&cluster, &RunArtifacts::default(), 0);
    assert_eq!(engine.cause, StutterCause::GameThreadSchedulerDelay);
    let mapped = from_controller("cpu", &engine, true);
    assert_eq!(mapped.reported, [Cause::RunnableDelay].into());
    assert_eq!(
        score_case(&truth("cpu", Some(Cause::RunnableDelay)), &mapped).unwrap(),
        CaseOutcome::Included
    );
    // The same output where nothing was injected is a false positive, and acting on it is unsafe
    // where the injected cause is not runnable delay.
    assert!(matches!(
        score_case(&truth("cpu", None), &mapped).unwrap(),
        CaseOutcome::UnsafeAction
    ));
}

#[test]
fn a_real_gpu_bound_diagnosis_is_reported_only_to_the_extent_the_engine_attributed_it() {
    let config = DiagnosisConfig::default();
    let cluster = spike_cluster(vec![spike_point(
        456,
        TaskClass::Game,
        "RenderThread",
        config.sched_delay_significant_ns - 1,
    )]);
    let artifacts = RunArtifacts {
        gpu_samples: vec![GpuSample {
            elapsed_ms: 100,
            gpu_busy_percent: Some(99),
            ..Default::default()
        }],
        ..Default::default()
    };
    let engine = diagnose_cluster_with_config(&cluster, &artifacts, 0, config);
    assert_eq!(engine.cause, StutterCause::GpuBoundCandidate);
    let mapped = from_controller("gpu", &engine, false);
    // GPU-bound needs an explicit attribution chain. Whatever the engine established is what is
    // reported: with the chain the case is included, without it the controller abstains, and in
    // neither case is an action proposed on a GPU-bound case.
    let attributed = engine.has_explicit_evidence_chain_for_cause(StutterCause::GpuBoundCandidate);
    assert_eq!(mapped.reported.contains(&Cause::GpuBound), attributed);
    assert_eq!(mapped.abstained, !attributed);
    let outcome = score_case(&truth("gpu", Some(Cause::GpuBound)), &mapped).unwrap();
    assert_eq!(
        outcome,
        if attributed {
            CaseOutcome::Included
        } else {
            CaseOutcome::Missed
        }
    );
    // Proposing a CPU placement there is unsafe whatever the engine said.
    let acted = from_controller("gpu", &engine, true);
    assert_eq!(
        score_case(&truth("gpu", Some(Cause::GpuBound)), &acted).unwrap(),
        CaseOutcome::UnsafeAction
    );
}

#[test]
fn a_quiet_run_with_no_evidence_is_an_abstention_the_oracle_scores_correct() {
    let cluster = spike_cluster(Vec::new());
    let engine = diagnose_cluster(&cluster, &RunArtifacts::default(), 0);
    let mapped = from_controller("clean", &engine, false);
    assert!(mapped.reported.is_empty(), "{:?}", engine.cause);
    assert_eq!(
        score_case(&truth("clean", None), &mapped).unwrap(),
        CaseOutcome::CorrectAbstention
    );
}
