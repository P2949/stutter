use super::*;
use crate::diagnosis::{Confidence, EvidenceChain, EvidenceChainKind};

fn controller(
    cause: StutterCause,
    secondary: &[StutterCause],
    chains: &[(EvidenceChainKind, bool)],
) -> ControllerDiagnosis {
    ControllerDiagnosis {
        cause,
        confidence: Confidence::Medium,
        causal_confidence: Default::default(),
        secondary_causes: secondary.to_vec(),
        evidence: Vec::new(),
        missing_evidence: Vec::new(),
        evidence_chains: chains
            .iter()
            .map(|(kind, explicit)| EvidenceChain {
                kind: *kind,
                explicit: *explicit,
                summary: String::new(),
                nodes: Vec::new(),
            })
            .collect(),
        primary: None,
        candidates: Vec::new(),
        candidate_rejections: Vec::new(),
        summary: String::new(),
    }
}

#[test]
fn the_controllers_causes_map_to_the_oracles_and_an_unmapped_or_unknown_one_reports_nothing() {
    let runnable = from_controller(
        "c1",
        &controller(StutterCause::CpuPressureCandidate, &[], &[]),
        true,
    );
    assert_eq!(runnable.reported, [Cause::RunnableDelay].into());
    assert!(runnable.action_proposed && !runnable.abstained);
    // Several scheduler-side causes are one oracle cause.
    let both = from_controller(
        "c2",
        &controller(
            StutterCause::GameThreadSchedulerDelay,
            &[StutterCause::CompositorSchedulerDelay],
            &[],
        ),
        false,
    );
    assert_eq!(both.reported, [Cause::RunnableDelay].into());
    // The unknown, and causes the oracle has no name for, are an abstention, never a guess.
    for cause in [
        StutterCause::Unknown,
        StutterCause::IrqDelayCandidate,
        StutterCause::RuntimeWaitCandidate,
    ] {
        let none = from_controller("c3", &controller(cause, &[], &[]), false);
        assert!(none.reported.is_empty() && none.abstained, "{cause:?}");
    }
    // Proposing an action is never an abstention.
    assert!(!from_controller("c4", &controller(StutterCause::Unknown, &[], &[]), true).abstained);
}

#[test]
fn a_cause_that_needs_an_attribution_chain_is_reported_only_with_one() {
    let without = from_controller(
        "g1",
        &controller(StutterCause::GpuBoundCandidate, &[], &[]),
        false,
    );
    assert!(without.reported.is_empty() && without.abstained);
    // A chain that is not explicit is not attribution either.
    let weak = from_controller(
        "g2",
        &controller(
            StutterCause::GpuBoundCandidate,
            &[],
            &[(EvidenceChainKind::Gpu, false)],
        ),
        false,
    );
    assert!(weak.reported.is_empty());
    let with = from_controller(
        "g3",
        &controller(
            StutterCause::GpuBoundCandidate,
            &[],
            &[(EvidenceChainKind::Gpu, true)],
        ),
        false,
    );
    assert_eq!(with.reported, [Cause::GpuBound].into());
    // The chain for another cause does not stand in.
    let wrong = from_controller(
        "g4",
        &controller(
            StutterCause::BlockIoCandidate,
            &[],
            &[(EvidenceChainKind::Gpu, true)],
        ),
        false,
    );
    assert!(wrong.reported.is_empty());
}

fn truth(id: &str, injected: Option<&str>) -> TruthRow {
    TruthRow {
        case_id: id.into(),
        injected: injected.map(str::to_owned),
    }
}

fn row(id: &str, at: u64, reported: &[&str], abstained: bool, acted: bool) -> DiagnosisRow {
    DiagnosisRow {
        case_id: id.into(),
        captured_unix: at,
        reported: reported.iter().map(|s| (*s).to_owned()).collect(),
        abstained,
        action_proposed: acted,
    }
}

/// Two causal positives found, a GPU-bound case left alone, and a clean negative control.
fn file() -> CaseFile {
    let truths = vec![
        truth("cpu", Some("runnable_delay")),
        truth("io", Some("io_pressure")),
        truth("gpu", Some("gpu_bound")),
        truth("clean", None),
    ];
    CaseFile {
        schema: 1,
        committed_unix: 100,
        truths_sha256: truths_digest(&truths),
        truths,
        diagnoses: vec![
            row("cpu", 200, &["runnable_delay"], false, true),
            row("io", 201, &["io_pressure"], false, false),
            row("gpu", 202, &["gpu_bound"], false, false),
            row("clean", 203, &[], true, false),
        ],
    }
}

#[test]
fn a_frozen_matched_case_file_is_scored_and_can_pass_the_gate() {
    let evaluation = evaluate(&file()).unwrap();
    assert_eq!(
        (
            evaluation.positives,
            evaluation.included,
            evaluation.negatives
        ),
        (3, 3, 1)
    );
    assert_eq!(evaluation.correct_abstentions, 1);
    assert_eq!(evaluation.unsafe_actions, 0);
    assert!(evaluation.gate_passed);
}

#[test]
fn ground_truth_must_be_frozen_first_and_the_cases_must_match_one_to_one() {
    let refused = |edit: &dyn Fn(&mut CaseFile), expected: &'static str| {
        let mut bad = file();
        edit(&mut bad);
        assert_eq!(evaluate(&bad).unwrap_err(), expected);
    };
    // A diagnosis captured before the truth was committed could have been read first.
    refused(
        &|f| f.diagnoses[0].captured_unix = 99,
        "OracleDiagnosisCapturedBeforeTheTruthWasFrozen",
    );
    // Truths edited after the commitment (here: a positive quietly turned into a negative).
    refused(
        &|f| f.truths[0].injected = None,
        "OracleTruthsNotTheCommittedOnes",
    );
    refused(&|f| f.schema = 2, "OracleSchemaUnsupported");
    // A case with no diagnosis, a duplicate, an extra diagnosis for no case, a made-up cause.
    refused(
        &|f| drop(f.diagnoses.pop()),
        "OracleCaseWithoutExactlyOneDiagnosis",
    );
    refused(
        &|f| {
            let again = f.diagnoses[0].clone();
            f.diagnoses.push(again);
        },
        "OracleCaseWithoutExactlyOneDiagnosis",
    );
    refused(
        &|f| f.diagnoses.push(row("extra", 300, &[], true, false)),
        "OracleDiagnosisForNoCase",
    );
    refused(
        &|f| f.diagnoses[1].reported = vec!["made_up".into()],
        "OracleUnknownCause",
    );
}

#[test]
fn an_unsafe_action_or_a_false_positive_is_counted_and_fails_the_gate() {
    let mut f = file();
    // Acting on the GPU-bound case: a CPU placement is never supported there.
    f.diagnoses[2] = row("gpu", 202, &["gpu_bound"], false, true);
    let evaluation = evaluate(&f).unwrap();
    assert_eq!(evaluation.unsafe_actions, 1);
    assert!(!evaluation.gate_passed);
    // Naming a cause on the negative control is a false positive.
    let mut g = file();
    g.diagnoses[3] = row("clean", 203, &["runnable_delay"], false, false);
    assert_eq!(evaluate(&g).unwrap().false_positive_cases, 1);
}

#[test]
fn a_study_with_no_case_file_says_so_and_the_newest_file_decides() {
    use std::os::unix::fs::PermissionsExt;
    let study = tempfile::tempdir().unwrap();
    assert_eq!(evaluate_study(study.path()).unwrap_err(), "NoOracleCases");
    let write = |name: &str, f: &CaseFile| {
        let path = study.path().join(name);
        std::fs::write(&path, serde_json::to_vec(f).unwrap()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    };
    write("oracle-cases-1.json", &file());
    assert!(evaluate_study(study.path()).unwrap().gate_passed);
    let mut newer = file();
    newer.committed_unix = 250;
    write("oracle-cases-2.json", &newer);
    assert_eq!(
        evaluate_study(study.path()).unwrap_err(),
        "OracleDiagnosisCapturedBeforeTheTruthWasFrozen"
    );
}
