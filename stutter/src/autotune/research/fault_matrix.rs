//! The receipt a fault matrix leaves, and the independent consumer that decides whether it can stand for
//! PLAN G6 (failure and security: single-use tool tickets, no wrong target, bounded cancellation, fault
//! fixtures, root tokens settled, the other process intact).
//!
//! The benign faults are injected on the real machine by an operator and are not run here; the
//! destructive ones (`DESTRUCTIVE_CLASSES`) are never injected into the real game and are proven by
//! fixtures. What belongs offline is the contract: the classes a matrix must cover, a typed receipt, and
//! a verifier that accepts G6 only from a receipt, bound to this very binary, in which every benign
//! class has a physical pass and every destructive class a fixture pass. A fixture never stands for a
//! benign class, a physical case for a destructive class is refused, a failed case blocks, and nothing
//! is inferred from a class that is absent.
//! class that is absent.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// The fault classes a G6 matrix must cover: the benign ones exercised on the real machine, the destructive
/// ones (`DESTRUCTIVE_CLASSES`) by fixtures.
pub const REQUIRED_CLASSES: [&str; 8] = [
    "wrong_target_refused",
    "single_use_ticket_replay_refused",
    "launch_cancellation_bounded",
    "unresolved_launch_retains_ownership",
    "worker_death_tokens_settled",
    "settings_rollback_after_crash",
    "other_process_intact_after_action",
    "gpu_health_halt_settles",
];

/// The classes whose injection into the real machine would itself be destructive (a worker killed with
/// tokens outstanding, a settings crash, a GPU hang): PLAN forbids running them against the real game, so
/// they are proven by fixtures and a physical case for one is refused. Every other class is benign and
/// needs a physical pass.
pub const DESTRUCTIVE_CLASSES: [&str; 3] = [
    "worker_death_tokens_settled",
    "settings_rollback_after_crash",
    "gpu_health_halt_settles",
];

/// The fixture that injects each destructive fault, by the name of its test: a worker killed with tokens outstanding, a
/// settings crash and a GPU reset, each against a fixture and never against the real game. A fixture case in a receipt
/// stands for its class only because one of these tests passes against the binary; a test pins that these tests exist, so the
/// mapping cannot name a fixture that was renamed or removed.
pub const FIXTURE_TESTS: [(&str, &str); 3] = [
    (
        "worker_death_tokens_settled",
        "a_worker_that_dies_after_the_prepare_is_an_error_with_the_token_already_journaled",
    ),
    (
        "settings_rollback_after_crash",
        "a_hook_recovers_every_arm_that_a_crash_left_edited",
    ),
    (
        "gpu_health_halt_settles",
        "a_gpu_reset_stops_the_whole_campaign_and_nothing_launches_again",
    ),
];

/// A receipt older than this no longer describes the binary that is deployed.
pub const MAX_AGE_SECONDS: u64 = 7 * 24 * 60 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// A simulated fault in a test: proves the code's handling, not the machine's.
    Fixture,
    /// A fault injected into the real system by an operator.
    Physical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Passed,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaultCase {
    pub class: String,
    pub mode: Mode,
    pub outcome: Outcome,
    /// SHA-256 (hex) of the raw record of the injection (the log or capture), so the case can be
    /// re-checked against it.
    pub evidence_sha256: String,
    pub observed_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaultMatrixReceipt {
    pub schema: u32,
    /// The Stutter binary the faults were injected into.
    pub binary_sha256: String,
    pub captured_unix: u64,
    pub cases: Vec<FaultCase>,
}

fn is_digest(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Why a receipt cannot stand for G6, or `Ok`.
pub fn attested(
    receipt: &FaultMatrixReceipt,
    own_binary_sha256: &str,
    now: u64,
) -> Result<(), &'static str> {
    if receipt.schema != 1 {
        return Err("FaultMatrixSchemaUnsupported");
    }
    if receipt.binary_sha256 != own_binary_sha256 {
        return Err("FaultMatrixForAnotherBinary");
    }
    if receipt.captured_unix > now.saturating_add(60)
        || now.saturating_sub(receipt.captured_unix) > MAX_AGE_SECONDS
    {
        return Err("FaultMatrixNotFresh");
    }
    if receipt
        .cases
        .iter()
        .any(|case| case.outcome == Outcome::Failed)
    {
        return Err("FaultCaseFailed");
    }
    for class in REQUIRED_CLASSES {
        let mut cases = receipt.cases.iter().filter(|case| case.class == class);
        let (Some(case), None) = (cases.next(), cases.next()) else {
            return Err("FaultClassMissingOrDuplicated");
        };
        if DESTRUCTIVE_CLASSES.contains(&class) {
            if case.mode == Mode::Physical {
                return Err("DestructiveFaultInjectedPhysically");
            }
        } else if case.mode != Mode::Physical {
            return Err("FaultClassOnlyFixtureTested");
        }
        if !is_digest(&case.evidence_sha256) {
            return Err("FaultCaseWithoutEvidence");
        }
        if case.observed_unix > receipt.captured_unix {
            return Err("FaultCaseAfterReceipt");
        }
    }
    Ok(())
}

/// The newest `fault-matrix-*.json` in a study directory, if one is there and readable.
pub fn read_latest(study: &Path) -> super::newest_record::Newest<FaultMatrixReceipt> {
    super::newest_record::newest(study, "fault-matrix-", |receipt: &FaultMatrixReceipt| {
        receipt.captured_unix
    })
}

/// The directory beside a receipt that holds the raw record each case names, one file per case called
/// `<evidence_sha256>.json`.
pub const EVIDENCE_DIR: &str = "fault-evidence";
const MAX_EVIDENCE_BYTES: u64 = 256 * 1024;

/// Every required case's named record is really there: a regular file (not a link) whose bytes hash to
/// the digest, that says it is this class, in this mode, with this outcome, for this binary, and that is
/// not shared with another case. Missing, modified, duplicated or misclassified evidence is refused.
pub fn artifacts_verified(receipt: &FaultMatrixReceipt, study: &Path) -> Result<(), &'static str> {
    let dir = study.join(EVIDENCE_DIR);
    let folder = match crate::syscall::open_dir_no_symlinks(&dir) {
        Ok(fd) => std::fs::File::from(fd),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err("FaultEvidenceMissing");
        }
        Err(_) => return Err("FaultEvidenceUnusable"),
    };
    let mut seen = std::collections::BTreeSet::new();
    for class in REQUIRED_CLASSES {
        let Some(case) = receipt.cases.iter().find(|case| case.class == class) else {
            return Err("FaultClassMissingOrDuplicated");
        };
        if !seen.insert(case.evidence_sha256.as_str()) {
            return Err("FaultEvidenceShared");
        }
        let name = format!("{}.json", case.evidence_sha256);
        let file = match crate::syscall::openat_nofollow(std::os::fd::AsFd::as_fd(&folder), &name) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err("FaultEvidenceMissing");
            }
            Err(_) => return Err("FaultEvidenceUnusable"),
        };
        let meta = file.metadata().map_err(|_| "FaultEvidenceUnusable")?;
        if !meta.is_file() || meta.len() > MAX_EVIDENCE_BYTES {
            return Err("FaultEvidenceUnusable");
        }
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(
            &mut std::io::Read::take(file, MAX_EVIDENCE_BYTES + 1),
            &mut bytes,
        )
        .map_err(|_| "FaultEvidenceUnusable")?;
        if stutter_core::hash::sha256_hex(&bytes) != case.evidence_sha256 {
            return Err("FaultEvidenceModified");
        }
        let record: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| "FaultEvidenceUnusable")?;
        let mode = match case.mode {
            Mode::Fixture => "fixture",
            Mode::Physical => "physical",
        };
        let outcome = match case.outcome {
            Outcome::Passed => "passed",
            Outcome::Failed => "failed",
        };
        if record["class"] != class
            || record["mode"] != mode
            || record["outcome"] != outcome
            || record["binary_sha256"] != receipt.binary_sha256
        {
            return Err("FaultEvidenceMisclassified");
        }
    }
    Ok(())
}

/// G6's status from the study directory: `Ok` when the newest receipt attests, otherwise the typed
/// reason (`NoFaultMatrixReceipt` when there is none).
pub fn gate_status(study: &Path, own_binary_sha256: &str, now: u64) -> Result<(), &'static str> {
    use super::newest_record::Newest;
    match read_latest(study) {
        Newest::None => Err("NoFaultMatrixReceipt"),
        Newest::Invalid => Err("FaultMatrixInvalid"),
        Newest::Found(receipt) => {
            attested(&receipt, own_binary_sha256, now)?;
            artifacts_verified(&receipt, study)
        }
    }
}

#[path = "fault_fixtures.rs"]
pub(crate) mod fault_fixtures;

#[cfg(test)]
#[path = "fault_matrix_tests.rs"]
mod tests;
