//! The producer side of the diagnosis oracle (PLAN P6). `oracle` scores a diagnosis against ground
//! truth but had nothing that produced either input from the real system. This module supplies both
//! halves and the rule that ties them: the controller's own `Diagnosis` is mapped to the oracle's
//! causes (a cause that needs an explicit attribution chain is reported only when it has one, so an
//! unknown is never graded as known); ground truth is committed by digest before any diagnosis is
//! captured; and a case file is accepted only if every capture is dated at or after that commitment,
//! the cases match one-to-one, and the digest is the truths'. Evaluating a file is read-only.
//!
//! What it cannot produce: the cases themselves. Injecting a cause and capturing the live controller's
//! output for it needs the real game and the real controller, so no case file exists yet and the
//! bundle says so instead of restating study statuses.

use std::{collections::BTreeSet, path::Path};

use serde::{Deserialize, Serialize};

use super::oracle::{Cause, Diagnosis as OracleDiagnosis, GroundTruth, gate_passes, summarize};
use crate::diagnosis::{Diagnosis as ControllerDiagnosis, StutterCause};

/// The oracle cause a controller cause stands for, `None` when the oracle has no such cause
/// (interrupt and runtime-wait delays, and the unknown).
fn oracle_cause(cause: StutterCause) -> Option<Cause> {
    match cause {
        StutterCause::CompositorSchedulerDelay
        | StutterCause::GameThreadSchedulerDelay
        | StutterCause::CpuPressureCandidate
        | StutterCause::CpuMonopolizationCandidate => Some(Cause::RunnableDelay),
        StutterCause::GpuBoundCandidate => Some(Cause::GpuBound),
        StutterCause::BlockIoCandidate => Some(Cause::IoPressure),
        StutterCause::IrqDelayCandidate
        | StutterCause::RuntimeWaitCandidate
        | StutterCause::Unknown => None,
    }
}

/// What the controller reported for one case, in the oracle's terms. A cause that needs an explicit
/// attribution chain and has none is not a report; abstaining means naming nothing and proposing
/// nothing.
pub fn from_controller(
    case_id: &str,
    diagnosis: &ControllerDiagnosis,
    action_proposed: bool,
) -> OracleDiagnosis {
    let reported: BTreeSet<Cause> = std::iter::once(diagnosis.cause)
        .chain(diagnosis.secondary_causes.iter().copied())
        .filter(|cause| {
            !cause.requires_explicit_attribution_chain()
                || diagnosis.has_explicit_evidence_chain_for_cause(*cause)
        })
        .filter_map(oracle_cause)
        .collect();
    OracleDiagnosis {
        case_id: case_id.to_owned(),
        abstained: reported.is_empty() && !action_proposed,
        reported,
        action_proposed,
    }
}

const CAUSES: [(&str, Cause); 5] = [
    ("runnable_delay", Cause::RunnableDelay),
    ("gpu_bound", Cause::GpuBound),
    ("cold_shader_cache", Cause::ColdShaderCache),
    ("io_pressure", Cause::IoPressure),
    ("owner_conflict", Cause::OwnerConflict),
];

fn cause_of(name: &str) -> Option<Cause> {
    CAUSES
        .iter()
        .find(|(known, _)| *known == name)
        .map(|(_, cause)| *cause)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TruthRow {
    pub case_id: String,
    /// `None`: a matched negative control with no injected cause.
    pub injected: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosisRow {
    pub case_id: String,
    pub captured_unix: u64,
    pub reported: Vec<String>,
    pub abstained: bool,
    pub action_proposed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaseFile {
    pub schema: u32,
    /// When the ground truth was frozen. No diagnosis may have been captured before this.
    pub committed_unix: u64,
    /// SHA-256 over the truths in case-id order.
    pub truths_sha256: String,
    pub truths: Vec<TruthRow>,
    pub diagnoses: Vec<DiagnosisRow>,
}

/// The digest a commitment to these truths has, independent of their order.
pub fn truths_digest(truths: &[TruthRow]) -> String {
    let mut sorted = truths.to_vec();
    sorted.sort_by(|a, b| a.case_id.cmp(&b.case_id));
    stutter_core::hash::sha256_hex(&serde_json::to_vec(&sorted).unwrap_or_default())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Evaluation {
    pub positives: usize,
    pub included: usize,
    pub negatives: usize,
    pub correct_abstentions: usize,
    pub false_positive_cases: usize,
    pub unsafe_actions: usize,
    /// The P6 gate: two causal positives included, a GPU-bound case left without action, and no unsafe
    /// action anywhere.
    pub gate_passed: bool,
}

/// Score a case file, or say why it cannot be scored.
pub fn evaluate(file: &CaseFile) -> Result<Evaluation, &'static str> {
    if file.schema != 1 {
        return Err("OracleSchemaUnsupported");
    }
    if truths_digest(&file.truths) != file.truths_sha256 {
        return Err("OracleTruthsNotTheCommittedOnes");
    }
    // Ground truth is fixed before the output is read: nothing may have been captured earlier.
    if file
        .diagnoses
        .iter()
        .any(|row| row.captured_unix < file.committed_unix)
    {
        return Err("OracleDiagnosisCapturedBeforeTheTruthWasFrozen");
    }
    let mut truths = Vec::new();
    for row in &file.truths {
        let injected = match row.injected.as_deref() {
            None => None,
            Some(name) => Some(cause_of(name).ok_or("OracleUnknownCause")?),
        };
        truths.push(GroundTruth {
            case_id: row.case_id.clone(),
            injected,
        });
    }
    // One diagnosis per case, matched by identity, in the truths' order.
    let mut diagnoses = Vec::new();
    for truth in &truths {
        let mut matching = file.diagnoses.iter().filter(|d| d.case_id == truth.case_id);
        let (Some(row), None) = (matching.next(), matching.next()) else {
            return Err("OracleCaseWithoutExactlyOneDiagnosis");
        };
        let mut reported = BTreeSet::new();
        for name in &row.reported {
            reported.insert(cause_of(name).ok_or("OracleUnknownCause")?);
        }
        diagnoses.push(OracleDiagnosis {
            case_id: row.case_id.clone(),
            reported,
            abstained: row.abstained,
            action_proposed: row.action_proposed,
        });
    }
    if file.diagnoses.len() != truths.len() {
        return Err("OracleDiagnosisForNoCase");
    }
    let summary = summarize(&truths, &diagnoses).map_err(|_| "OracleCasesInconsistent")?;
    let gate_passed = gate_passes(&truths, &diagnoses).map_err(|_| "OracleCasesInconsistent")?;
    Ok(Evaluation {
        positives: summary.positives,
        included: summary.included,
        negatives: summary.negatives,
        correct_abstentions: summary.correct_abstentions,
        false_positive_cases: summary.false_positive_cases,
        unsafe_actions: summary.unsafe_actions,
        gate_passed,
    })
}

/// The ground truth, frozen and written BEFORE any case is run: a file of its own that nothing later edits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TruthCommitment {
    pub schema: u32,
    pub intent_sha256: String,
    pub committed_unix: u64,
    pub truths_sha256: String,
    pub truths: Vec<TruthRow>,
}

/// What the controller reported, captured AFTER the commitment in a second file. It names the exact bytes of the
/// commitment it was captured against, so changing the truths afterwards cannot go unnoticed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capture {
    pub schema: u32,
    pub intent_sha256: String,
    pub captured_unix: u64,
    /// SHA-256 of the commitment file's bytes as the controller saw them.
    pub truth_commitment_sha256: String,
    pub diagnoses: Vec<DiagnosisRow>,
}

/// Score the newest commitment against the newest capture, read-only and independently: both files are
/// discovered strictly, the capture must be bound to this very commitment and campaign, every diagnosis dated
/// after the commitment, and the cases must match one for one. `Err` carries why not.
pub fn evaluate_study(study: &Path, intent_sha256: &str) -> Result<Evaluation, &'static str> {
    use super::newest_record::{Newest, newest_with_bytes};
    let (commitment, commitment_bytes) =
        match newest_with_bytes::<TruthCommitment>(study, "oracle-truth-", |c| c.committed_unix) {
            Newest::None => return Err("NoOracleCases"),
            Newest::Invalid => return Err("OracleCasesInvalid"),
            Newest::Found(found) => found,
        };
    let capture = match newest_with_bytes::<Capture>(study, "oracle-capture-", |c| c.captured_unix)
    {
        Newest::None => return Err("NoOracleCapture"),
        Newest::Invalid => return Err("OracleCasesInvalid"),
        Newest::Found((capture, _)) => capture,
    };
    if commitment.schema != 2 || capture.schema != 2 {
        return Err("OracleSchemaUnsupported");
    }
    if commitment.intent_sha256 != intent_sha256 || capture.intent_sha256 != intent_sha256 {
        return Err("OracleForAnotherCampaign");
    }
    if capture.truth_commitment_sha256 != stutter_core::hash::sha256_hex(&commitment_bytes) {
        return Err("OracleCaptureNotBoundToTheCommittedTruths");
    }
    if capture.captured_unix < commitment.committed_unix {
        return Err("OracleDiagnosisCapturedBeforeTheTruthWasFrozen");
    }
    evaluate(&CaseFile {
        schema: 1,
        committed_unix: commitment.committed_unix,
        truths_sha256: commitment.truths_sha256,
        truths: commitment.truths,
        diagnoses: capture.diagnoses,
    })
}

#[cfg(test)]
#[path = "oracle_cases_tests.rs"]
mod tests;
