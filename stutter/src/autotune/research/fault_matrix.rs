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
pub fn read_latest(study: &Path) -> Option<FaultMatrixReceipt> {
    let mut best: Option<FaultMatrixReceipt> = None;
    for entry in std::fs::read_dir(study).ok()?.flatten() {
        let name = entry.file_name();
        let name = name.to_str()?;
        if !(name.starts_with("fault-matrix-") && name.ends_with(".json")) {
            continue;
        }
        let Ok(bytes) = std::fs::read(entry.path()) else {
            continue;
        };
        if let Ok(candidate) = serde_json::from_slice::<FaultMatrixReceipt>(&bytes)
            && best
                .as_ref()
                .is_none_or(|current| candidate.captured_unix > current.captured_unix)
        {
            best = Some(candidate);
        }
    }
    best
}

/// G6's status from the study directory: `Ok` when the newest receipt attests, otherwise the typed
/// reason (`NoFaultMatrixReceipt` when there is none).
pub fn gate_status(study: &Path, own_binary_sha256: &str, now: u64) -> Result<(), &'static str> {
    match read_latest(study) {
        None => Err("NoFaultMatrixReceipt"),
        Some(receipt) => attested(&receipt, own_binary_sha256, now),
    }
}

#[cfg(test)]
#[path = "fault_matrix_tests.rs"]
mod tests;
