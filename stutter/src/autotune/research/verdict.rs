//! The separate statuses of a finished campaign's verification (PLAN section 9). A report that
//! reconstructs deterministically is not thereby backed by its raw files, backed raw files are not
//! thereby a qualified Proton source, and none of that is a restored system. Each is its own
//! boolean computed from its own evidence; `final_gate_qualified` is true only when all are, and
//! the disposition names the strongest thing that is actually established.

use serde::Serialize;

use super::{
    child_ledger::Disposition, loaded_runtime::RuntimeBinding, raw_verify::RawVerification,
    report::AttemptEvidenceV1,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Verdict {
    /// The report is the deterministic function of the sealed journal and evidence records.
    pub reconstruction_passed: bool,
    /// Every completed attempt's native files were reopened and matched their sealed hashes.
    pub raw_files_verified: bool,
    /// Every completed attempt's mapped runtime was matched to its own arm's installation.
    pub loaded_runtime_qualified: bool,
    /// Every planned child study reached an acceptable terminal disposition: qualified, or a legitimate
    /// feature exclusion (`gate_verdict::legitimately_excluded`); nothing else may stand in for one.
    pub child_studies_qualified: bool,
    /// Every child study's own report reconstructs from its sealed records and its raw files replay.
    pub child_reports_verified: bool,
    /// Worker restoration was physically proven (no verifier of this exists yet: always the
    /// caller's observation, never inferred from the other statuses).
    pub restoration_verified: bool,
    pub final_gate_qualified: bool,
    /// The campaign's own sealed evidence qualifies (every study-backed fact and gates G2-G5, G7, G9),
    /// whatever the host looks like now. A later reboot or package change can move `final_gate_qualified`
    /// through G0, G1, G6 and G8; it cannot move this.
    pub historical_evidence_qualified: bool,
    /// `QualifiedEndToEnd` when every study qualified; otherwise what stood in for a qualification (a feature
    /// no build supports, or one whose function the game cannot report) or a justified abstention.
    pub qualification_scope: &'static str,
    /// Facts the historical result still cannot take from sealed evidence.
    pub historical_notes: Vec<&'static str>,
    /// `AbortedHealth`, `Qualified`, `PartiallyQualified` or `Unverified`.
    pub disposition: &'static str,
    /// One verdict per PLAN gate, each from its own evidence (`gate_verdict`).
    pub gates: Vec<super::gate_verdict::GateVerdict>,
}

pub struct VerdictInputs<'a> {
    pub reconstruction_passed: bool,
    pub raw: Option<&'a RawVerification>,
    pub evidence: &'a [AttemptEvidenceV1],
    pub child_studies: &'a [Disposition],
    pub child_reports_verified: bool,
    /// The campaign ran as a child-study funnel: its attempts are in the child studies (the flat
    /// parent schedule is empty), so raw files and loaded runtimes are judged from the children.
    pub funnel: bool,
    /// Every completed child attempt's mapped runtime matched its own arm (funnel campaigns).
    pub child_runtime_qualified: bool,
    /// (attempts with an eligible, game-bound frame-source receipt, completed attempts) over the
    /// children; `None` when no child completed an attempt.
    pub child_telemetry: Option<(u32, u32)>,
    /// Whether every holdout reproduced its confirmation (`None`: no holdout was judged).
    pub child_replication: Option<bool>,
    /// G6: the fault-matrix receipt status (see `fault_matrix::gate_status`).
    pub fault_matrix: Result<(), &'static str>,
    /// G5: the owned-pressure control receipt status (`pressure_control::status`); only consulted when an
    /// action study ended as a justified no-op.
    pub pressure_control: Result<(), &'static str>,
    /// G3: the mandatory resident observation status over the children (`ChildrenVerification::resident_summary`).
    pub child_resident: Result<(), &'static str>,
    /// G2: the player's saves and cloud copy are unchanged since before the campaign (`protected_state`).
    pub protected_state: Result<(), &'static str>,
    /// G0: the kernel, CPU topology and GPU are the ones recorded before the campaign (`host_hardware`).
    pub hardware: Result<(), &'static str>,
    /// G1: the control socket and its directory as inspected read-only (`control_socket`).
    pub control_socket: Option<super::control_socket::ControlSocketFacts>,
    /// G8: the deployed binary is attested by a receipt (see `deploy_receipt`).
    pub deployment_attested: bool,
    /// G1: what a read-only inspection of the campaign's lease found.
    pub lease: Option<super::gate_verdict::LeaseFacts>,
    /// G0: the host as read now against what the campaign froze.
    pub host: Option<super::gate_verdict::HostFacts>,
    pub restoration_verified: bool,
    /// An unacknowledged GPU-health incident is on record.
    pub health_blocked: bool,
}

/// What the qualified studies of a campaign add up to: the plan distinguishes a full end-to-end qualification
/// from one with excluded optional features or a justified abstention.
fn qualification_scope(studies: &[Disposition]) -> &'static str {
    use super::{child_study::StudyResult, gate_verdict};
    if studies
        .iter()
        .any(|s| s.result == Some(StudyResult::Unqualified) && s.reason == "FeatureNotObservable")
    {
        "QualifiedWithUnobservedFeatures"
    } else if studies.iter().any(gate_verdict::legitimately_excluded) {
        "QualifiedWithExcludedOptionalFeatures"
    } else if studies.iter().any(gate_verdict::not_indicated) {
        "QualifiedWithJustifiedAbstention"
    } else {
        "QualifiedEndToEnd"
    }
}

pub fn judge(inputs: &VerdictInputs<'_>) -> Verdict {
    judge_inner(inputs, false)
}

/// Judge the scientific evidence while the caller still owns its restoring lease.
pub(crate) fn judge_preparing(inputs: &VerdictInputs<'_>) -> Verdict {
    judge_inner(inputs, true)
}

fn judge_inner(inputs: &VerdictInputs<'_>, preparing: bool) -> Verdict {
    let raw_files_verified = if inputs.funnel {
        inputs.child_reports_verified
    } else {
        inputs.raw.is_some_and(RawVerification::passed)
    };
    let with_native: Vec<_> = inputs
        .evidence
        .iter()
        .filter_map(|item| item.native_completion.as_deref())
        .collect();
    let loaded_runtime_qualified = if inputs.funnel {
        inputs.child_runtime_qualified
    } else {
        !with_native.is_empty()
            && with_native
                .iter()
                .all(|native| native.runtime_binding == Some(RuntimeBinding::Matched))
    };
    let child_studies_qualified = !inputs.child_studies.is_empty()
        && inputs
            .child_studies
            .iter()
            .all(super::gate_verdict::resolved);
    let gate_inputs = super::gate_verdict::GateInputs {
        raw_files_verified,
        loaded_runtime_qualified,
        child_studies: inputs.child_studies,
        child_reports_verified: inputs.child_reports_verified,
        restoration_verified: inputs.restoration_verified,
        health_blocked: inputs.health_blocked,
        telemetry: inputs.child_telemetry,
        replication: inputs.child_replication,
        fault_matrix: inputs.fault_matrix,
        pressure_control: inputs.pressure_control,
        resident_observation: inputs.child_resident,
        protected_state: inputs.protected_state,
        hardware: inputs.hardware,
        control_socket: inputs.control_socket,
        deployment_attested: inputs.deployment_attested,
        lease: inputs.lease,
        host: inputs.host,
    };
    let gates = if preparing {
        super::gate_verdict::gates_preparing(&gate_inputs)
    } else {
        super::gate_verdict::gates(&gate_inputs)
    };
    // The overall result is the conjunction of the study-backed facts and every gate: a campaign
    // whose host baseline, lease receipts, telemetry, fault matrix or deployment receipt is not attested
    // cannot be `Qualified`, however clean its studies are.
    let final_gate_qualified = inputs.reconstruction_passed
        && raw_files_verified
        && loaded_runtime_qualified
        && child_studies_qualified
        && inputs.child_reports_verified
        && inputs.restoration_verified
        && !inputs.health_blocked
        && gates.iter().all(|gate| gate.accepted);
    // The historical result rests on the campaign's own sealed evidence only. The gates are recomputed with the
    // facts that describe today rather than the campaign: a later GPU incident (`health_blocked`), the player's
    // saves as they are now (`protected_state`) and the age of the control receipt cannot move it. Sealing the
    // protected-state comparison at finalization is not yet done, which `historical_notes` says.
    let historical_gate_inputs = super::gate_verdict::GateInputs {
        health_blocked: false,
        protected_state: Ok(()),
        pressure_control: match inputs.pressure_control {
            Err("PressureControlNotFresh") => Ok(()),
            other => other,
        },
        ..gate_inputs
    };
    let historical_gates = super::gate_verdict::gates_preparing(&historical_gate_inputs);
    let historical_evidence_qualified = inputs.reconstruction_passed
        && raw_files_verified
        && loaded_runtime_qualified
        && child_studies_qualified
        && inputs.child_reports_verified
        && inputs.restoration_verified
        && historical_gates
            .iter()
            .filter(|gate| ["G2", "G3", "G4", "G5", "G7", "G9"].contains(&gate.gate))
            .all(|gate| gate.accepted);
    let qualification_scope = qualification_scope(inputs.child_studies);
    let disposition = if inputs.health_blocked {
        "AbortedHealth"
    } else if final_gate_qualified {
        "Qualified"
    } else if inputs.reconstruction_passed && raw_files_verified {
        "PartiallyQualified"
    } else {
        "Unverified"
    };
    Verdict {
        reconstruction_passed: inputs.reconstruction_passed,
        raw_files_verified,
        loaded_runtime_qualified,
        child_studies_qualified,
        child_reports_verified: inputs.child_reports_verified,
        restoration_verified: inputs.restoration_verified,
        final_gate_qualified,
        historical_evidence_qualified,
        qualification_scope,
        historical_notes: vec!["ProtectedStateNotSealedAtFinalization"],
        disposition,
        gates,
    }
}

#[cfg(test)]
#[path = "verdict_tests.rs"]
mod tests;
