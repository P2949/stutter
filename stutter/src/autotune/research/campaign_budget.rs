//! The campaign's cumulative budgets across its child studies. Each child study runs under its own
//! journalled `Driver`, which starts counting active time and evidence bytes from zero; the budgets
//! the operator authorized (`max_active_seconds`, `max_evidence_bytes`) belong to the whole campaign.
//! When a child ends it records what it used beside its study, and the next child starts from the
//! total already spent, so a sequence of children cannot each respect a limit while together
//! exceeding it. A child directory with no record (a crash, or a study still running when the
//! controller stopped) has an unknown cost and is charged the whole active budget: nothing further
//! runs on an unknown spend.

use std::{fs::File, os::fd::AsFd};

use serde::{Deserialize, Serialize};

use super::report_store::{evidence_cost, read_evidence, read_private, write_new};

const FILE: &str = "budget.json";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spent {
    pub active_seconds: u64,
    pub evidence_bytes: u64,
}

impl Spent {
    pub fn plus(self, other: Self) -> Self {
        Self {
            active_seconds: self.active_seconds.saturating_add(other.active_seconds),
            evidence_bytes: self.evidence_bytes.saturating_add(other.evidence_bytes),
        }
    }
}

/// Record what one finished child used. An existing record must be identical.
pub fn record(study: &File, evidence: &File, uid: u32, active_seconds: u64) -> anyhow::Result<()> {
    let mut evidence_bytes = 0u64;
    for item in read_evidence(evidence, uid)?.values() {
        evidence_bytes = evidence_bytes.saturating_add(evidence_cost(item)?);
    }
    let spent = Spent {
        active_seconds,
        evidence_bytes,
    };
    let bytes = serde_json::to_vec(&spent)?;
    match write_new(study, FILE, &bytes) {
        Ok(()) => Ok(()),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|io| io.kind() == std::io::ErrorKind::AlreadyExists) =>
        {
            anyhow::ensure!(
                read_private(study, FILE, uid)? == bytes,
                "the recorded campaign budget differs"
            );
            Ok(())
        }
        Err(error) => Err(error),
    }
}

/// What the campaign's earlier children have spent: the sum of their records, with an unrecorded
/// child charged the whole `max_active_seconds`.
pub fn spent(studies: &File, uid: u32, max_active_seconds: u64) -> anyhow::Result<Spent> {
    let mut total = Spent::default();
    for entry in std::fs::read_dir(format!(
        "/proc/self/fd/{}",
        std::os::fd::AsRawFd::as_raw_fd(studies)
    ))? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        let dir = File::from(crate::syscall::openat_dir_nofollow(studies.as_fd(), &name)?);
        let one = read_private(&dir, FILE, uid)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Spent>(&bytes).ok())
            .unwrap_or(Spent {
                active_seconds: max_active_seconds,
                evidence_bytes: 0,
            });
        // The raw archive and receipts beside the records occupy the same cap; admission counts what is on disk now,
        // the same way the final replay does, so a child is never admitted into a cap the archive already used.
        let one = Spent {
            evidence_bytes: one.evidence_bytes.saturating_add(sidecar_bytes(&dir, 0)?),
            ..one
        };
        total = total.plus(one);
    }
    Ok(total)
}

/// Independent replay of the campaign's cumulative budgets from what its children sealed: every child
/// has a spend record, its evidence bytes equal what its own sealed evidence costs, and the totals (and
/// the attempts the ledger counted) stay within what the operator authorized. `None` is a clean replay;
/// otherwise the first problem. The active seconds are the child's own record (they cannot be rebuilt
/// from sealed bytes) and are bounded, not recomputed.
pub fn replay(
    studies: &File,
    uid: u32,
    parent: &super::ResearchIntentV1,
    attempts: &std::collections::BTreeMap<String, u32>,
) -> Option<String> {
    let outcome = (|| -> anyhow::Result<Option<String>> {
        let mut total = Spent::default();
        let mut sidecars = 0u64;
        for entry in std::fs::read_dir(format!(
            "/proc/self/fd/{}",
            std::os::fd::AsRawFd::as_raw_fd(studies)
        ))? {
            let name = entry?.file_name().to_string_lossy().into_owned();
            let dir = File::from(crate::syscall::openat_dir_nofollow(studies.as_fd(), &name)?);
            let Ok(bytes) = read_private(&dir, FILE, uid) else {
                return Ok(Some(format!("BudgetRecordMissing:{name}")));
            };
            let recorded: Spent = serde_json::from_slice(&bytes)?;
            let evidence = File::from(crate::syscall::openat_dir_nofollow(
                dir.as_fd(),
                "evidence",
            )?);
            let mut actual = 0u64;
            for item in read_evidence(&evidence, uid)?.values() {
                actual = actual.saturating_add(evidence_cost(item)?);
            }
            if actual != recorded.evidence_bytes {
                return Ok(Some(format!("BudgetEvidenceBytesMismatch:{name}")));
            }
            // Active time cannot be rebuilt from sealed bytes, but each completed attempt witnessed its own start and
            // end (the native completion's clock or the long session's record): a record that claims less than
            // that was understated.
            let witnessed: u64 =
                super::child_verify_rules::processes_of(&read_evidence(&evidence, uid)?)
                    .iter()
                    .map(|p| p.completed_unix.saturating_sub(p.started_unix))
                    .fold(0u64, u64::saturating_add);
            if recorded.active_seconds < witnessed {
                return Ok(Some(format!("BudgetActiveSecondsUnderstated:{name}")));
            }
            // What is on disk beside the sealed records (the raw archive, receipts) counts toward the
            // same cap, measured now rather than read from a record.
            sidecars = sidecars.saturating_add(sidecar_bytes(&dir, 0)?);
            total = total.plus(recorded);
        }
        if total.active_seconds > u64::from(parent.max_active_seconds) {
            return Ok(Some("CampaignActiveSecondsExceeded".into()));
        }
        if total.evidence_bytes.saturating_add(sidecars) > parent.max_evidence_bytes {
            return Ok(Some("CampaignEvidenceBytesExceeded".into()));
        }
        if attempts.values().map(|n| u64::from(*n)).sum::<u64>() > u64::from(parent.max_attempts) {
            return Ok(Some("CampaignAttemptsExceeded".into()));
        }
        Ok(None)
    })();
    outcome.unwrap_or_else(|error| Some(format!("BudgetReplayFailed:{error:#}")))
}

/// The bytes every regular file under a study directory occupies besides its sealed evidence records
/// and journal (which the budget already counts): the raw archive and every sidecar receipt. Entries
/// are opened through directory descriptors without following links; a link, a special file or a tree
/// that is too deep is an error rather than a smaller number.
fn sidecar_bytes(dir: &File, depth: u32) -> anyhow::Result<u64> {
    anyhow::ensure!(depth <= 8, "study tree too deep");
    let base = format!("/proc/self/fd/{}", std::os::fd::AsRawFd::as_raw_fd(dir));
    let mut total = 0u64;
    for entry in std::fs::read_dir(&base)? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if depth == 0 && matches!(name.as_str(), "evidence" | "journal") {
            continue;
        }
        let kind = std::fs::symlink_metadata(format!("{base}/{name}"))?.file_type();
        if kind.is_file() {
            let file = crate::syscall::openat_nofollow(dir.as_fd(), &name)?;
            total = total.saturating_add(file.metadata()?.len());
        } else if kind.is_dir() {
            let child = File::from(crate::syscall::openat_dir_nofollow(dir.as_fd(), &name)?);
            total = total.saturating_add(sidecar_bytes(&child, depth + 1)?);
        } else {
            anyhow::bail!("unexpected file type: {name}");
        }
    }
    Ok(total)
}

#[cfg(test)]
#[path = "campaign_budget_tests.rs"]
mod tests;
