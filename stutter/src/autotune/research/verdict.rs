//! The separate statuses of a finished campaign's verification (PLAN section 9). A report that
//! reconstructs deterministically is not thereby backed by its raw files, backed raw files are not
//! thereby a qualified Proton source, and none of that is a restored system. Each is its own
//! boolean computed from its own evidence; `final_gate_qualified` is true only when all are, and
//! the disposition names the strongest thing that is actually established.

use serde::Serialize;

use super::{
    child_ledger::Disposition, child_study::StudyResult, loaded_runtime::RuntimeBinding,
    raw_verify::RawVerification, report::AttemptEvidenceV1,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Verdict {
    /// The report is the deterministic function of the sealed journal and evidence records.
    pub reconstruction_passed: bool,
    /// Every completed attempt's native files were reopened and matched their sealed hashes.
    pub raw_files_verified: bool,
    /// Every completed attempt's mapped runtime was matched to its own arm's installation.
    pub loaded_runtime_qualified: bool,
    /// Every planned child study ran to a qualified result.
    pub child_studies_qualified: bool,
    /// Every child study's own report reconstructs from its sealed records and its raw files replay.
    pub child_reports_verified: bool,
    /// Worker restoration was physically proven (no verifier of this exists yet: always the
    /// caller's observation, never inferred from the other statuses).
    pub restoration_verified: bool,
    pub final_gate_qualified: bool,
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

pub fn judge(inputs: &VerdictInputs<'_>) -> Verdict {
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
            .all(|study| study.result == Some(StudyResult::Qualified));
    let gates = super::gate_verdict::gates(&super::gate_verdict::GateInputs {
        raw_files_verified,
        loaded_runtime_qualified,
        child_studies: inputs.child_studies,
        child_reports_verified: inputs.child_reports_verified,
        restoration_verified: inputs.restoration_verified,
        health_blocked: inputs.health_blocked,
        telemetry: inputs.child_telemetry,
        replication: inputs.child_replication,
        deployment_attested: inputs.deployment_attested,
        lease: inputs.lease,
        host: inputs.host,
    });
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
        disposition,
        gates,
    }
}

#[cfg(test)]
#[path = "verdict_tests.rs"]
mod tests;
