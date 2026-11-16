//! Read-only replay of a finished campaign's raw native evidence. `report_store::load_and_verify`
//! proves the report is the deterministic function of the sealed journal and evidence records; this
//! goes one step further. It binds every `Completed` journal record to an evidence record with the
//! sealed digest, reopens the native result files that record names, re-hashes them and re-parses
//! the summary and frame file, so a record whose files were edited, removed, replaced or never
//! matched cannot pass as verified, and one valid result cannot vouch for an incomplete campaign.
//! Nothing is written.

use std::{fs::File, path::PathBuf};

use serde::Serialize;

use super::{
    ProtonSource,
    cyberpunk::reverify_completion,
    journal::{AttemptOutcome, AttemptRecord},
    report_store::read_evidence,
};

/// One attempt whose raw evidence did not replay.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RawProblem {
    pub slot_index: u32,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RawVerification {
    /// `Completed` journal records in the campaign.
    pub completed: u32,
    /// Completed attempts whose sealed native files were reopened and matched.
    pub verified: u32,
    /// Attempts the journal sealed as failed (they carry no native result to replay).
    pub failed_attempts: u32,
    pub problems: Vec<RawProblem>,
}

impl RawVerification {
    /// Every completed attempt replayed, and at least one existed.
    pub fn passed(&self) -> bool {
        self.problems.is_empty() && self.completed > 0 && self.verified == self.completed
    }
}

/// Replay every completed attempt's native files under the arm's own results directory.
pub fn verify_raw_evidence(
    evidence_dir: &File,
    uid: u32,
    records: &[AttemptRecord],
    results_for: &dyn Fn(ProtonSource) -> PathBuf,
) -> anyhow::Result<RawVerification> {
    let evidence = read_evidence(evidence_dir, uid)?;
    let mut report = RawVerification {
        completed: 0,
        verified: 0,
        failed_attempts: 0,
        problems: Vec::new(),
    };
    for record in records {
        let slot_index = record.slot.index;
        let AttemptOutcome::Completed { evidence_sha256 } = &record.outcome else {
            report.failed_attempts += 1;
            if evidence.contains_key(&slot_index) {
                report.problems.push(RawProblem {
                    slot_index,
                    reason: "a failed attempt has evidence".into(),
                });
            }
            continue;
        };
        report.completed += 1;
        let outcome = (|| {
            let item = evidence
                .get(&slot_index)
                .ok_or_else(|| anyhow::anyhow!("completed attempt has no evidence record"))?;
            anyhow::ensure!(
                item.digest()? == *evidence_sha256,
                "evidence differs from the digest the journal sealed"
            );
            let completion = item
                .native_completion
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("completed attempt has no native result"))?;
            anyhow::ensure!(
                completion.process.uid == uid,
                "sealed native result belongs to another user"
            );
            anyhow::ensure!(
                completion.proton_source == record.slot.arm,
                "native result belongs to another Proton arm"
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
    // Evidence for a slot the journal never sealed is as wrong as a missing record.
    for slot_index in evidence.keys() {
        if !records
            .iter()
            .any(|record| record.slot.index == *slot_index)
        {
            report.problems.push(RawProblem {
                slot_index: *slot_index,
                reason: "evidence has no sealed journal record".into(),
            });
        }
    }
    Ok(report)
}

#[cfg(test)]
#[path = "raw_verify_tests.rs"]
mod tests;
