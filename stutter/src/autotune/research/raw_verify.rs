//! Read-only replay of a finished campaign's raw native evidence. `report_store::load_and_verify`
//! proves the report is the deterministic function of the sealed journal and evidence records; this
//! goes one step further and reopens the native result files those records name, re-hashes them and
//! re-parses the summary and frame file, so a record whose files were edited, removed, replaced or
//! never matched cannot pass as verified. Nothing is written.

use std::{fs::File, path::PathBuf};

use serde::Serialize;

use super::{ProtonSource, cyberpunk::reverify_completion, report_store::read_evidence};

/// One attempt whose raw evidence did not replay.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RawProblem {
    pub slot_index: u32,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RawVerification {
    /// Attempts whose sealed native files were reopened and matched.
    pub verified: u32,
    /// Attempts with no native completion at all (a failed attempt carries no raw result).
    pub without_native_result: u32,
    pub problems: Vec<RawProblem>,
}

impl RawVerification {
    /// Every sealed native result replayed, and at least one existed.
    pub fn passed(&self) -> bool {
        self.problems.is_empty() && self.verified > 0
    }
}

/// Replay every sealed attempt's native files under the arm's own results directory.
pub fn verify_raw_evidence(
    evidence_dir: &File,
    uid: u32,
    results_for: &dyn Fn(ProtonSource) -> PathBuf,
) -> anyhow::Result<RawVerification> {
    let mut report = RawVerification {
        verified: 0,
        without_native_result: 0,
        problems: Vec::new(),
    };
    for (slot_index, item) in read_evidence(evidence_dir, uid)? {
        let Some(completion) = item.native_completion.as_deref() else {
            report.without_native_result += 1;
            continue;
        };
        let outcome = (|| {
            anyhow::ensure!(
                completion.process.uid == uid,
                "sealed native result belongs to another user"
            );
            anyhow::ensure!(
                (item.average_fps - completion.metrics.average_fps).abs() <= f64::EPSILON,
                "attempt evidence FPS differs from the sealed native metrics"
            );
            reverify_completion(&results_for(completion.proton_source), completion)
        })();
        match outcome {
            Ok(()) => report.verified += 1,
            Err(error) => report.problems.push(RawProblem {
                slot_index,
                reason: format!("{error:#}"),
            }),
        }
    }
    Ok(report)
}

#[cfg(test)]
#[path = "raw_verify_tests.rs"]
mod tests;
