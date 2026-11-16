//! Durable, restart-safe record of the child-study funnel. Every decision about a planned study
//! (qualified, unqualified, failed, deferred for budget, blocked) is one immutable, hash-chained
//! record in the campaign directory, so a restarted controller resumes at the first study without a
//! record, a tampered or reordered ledger is refused, and no study is silently dropped or run twice.

use std::{collections::BTreeMap, fs::File};

use serde::{Deserialize, Serialize};

use super::{
    child_study::{ChildStudy, NextStudy, Profile, Stage, StudyResult, next_study},
    report_store::{read_private, write_new},
};

const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const MAX_RECORDS: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    seq: u32,
    study_sha256: String,
    result: StudyResult,
    /// Machine-readable cause (for example `BlockedProfileApplier`), empty when none applies.
    reason: String,
    /// SHA-256 of the previous record's bytes (all zeros for the first).
    prev_sha256: String,
}

/// One planned study and what became of it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Disposition {
    pub study_sha256: String,
    pub profile: Profile,
    pub stage: Stage,
    /// `None` means the study has no decision yet (never started).
    pub result: Option<StudyResult>,
    pub reason: String,
}

pub struct ChildLedger {
    dir: File,
    plan: Vec<ChildStudy>,
    results: BTreeMap<String, (StudyResult, String)>,
    tip: String,
    next_seq: u32,
}

fn name(seq: u32) -> String {
    format!("{seq:04}.json")
}

impl ChildLedger {
    /// Open the ledger directory and verify every record: canonical names without gaps, a valid
    /// hash chain, only planned studies and at most one decision per study.
    pub fn open(dir: File, uid: u32, plan: Vec<ChildStudy>) -> anyhow::Result<Self> {
        let mut seqs: Vec<u32> = Vec::new();
        for entry in std::fs::read_dir(format!(
            "/proc/self/fd/{}",
            std::os::fd::AsRawFd::as_raw_fd(&dir)
        ))? {
            let file = entry?
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("non-UTF8 ledger entry"))?;
            let seq = file
                .strip_suffix(".json")
                .and_then(|stem| stem.parse::<u32>().ok())
                .filter(|seq| file == name(*seq))
                .ok_or_else(|| anyhow::anyhow!("unexpected ledger entry"))?;
            seqs.push(seq);
            anyhow::ensure!(seqs.len() <= MAX_RECORDS, "excessive ledger");
        }
        seqs.sort_unstable();
        let mut ledger = Self {
            dir,
            plan,
            results: BTreeMap::new(),
            tip: GENESIS.to_owned(),
            next_seq: 0,
        };
        for (expected, seq) in seqs.into_iter().enumerate() {
            anyhow::ensure!(seq as usize == expected, "gap in the ledger");
            let bytes = read_private(&ledger.dir, &name(seq), uid)?;
            let record: Record = serde_json::from_slice(&bytes)?;
            anyhow::ensure!(
                record.seq == seq && record.prev_sha256 == ledger.tip,
                "ledger chain is broken"
            );
            anyhow::ensure!(
                ledger
                    .plan
                    .iter()
                    .any(|study| study.study_sha256 == record.study_sha256),
                "ledger names a study that is not in the plan"
            );
            anyhow::ensure!(
                ledger
                    .results
                    .insert(record.study_sha256, (record.result, record.reason))
                    .is_none(),
                "a study has two decisions"
            );
            ledger.tip = stutter_core::hash::sha256_hex(&bytes);
            ledger.next_seq = seq + 1;
        }
        Ok(ledger)
    }

    /// Record one decision. A study decided already, or not in the plan, is refused.
    pub fn record(
        &mut self,
        study: &ChildStudy,
        result: StudyResult,
        reason: &str,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.plan
                .iter()
                .any(|planned| planned.study_sha256 == study.study_sha256),
            "study is not in the plan"
        );
        anyhow::ensure!(
            !self.results.contains_key(&study.study_sha256),
            "study already has a decision"
        );
        let record = Record {
            seq: self.next_seq,
            study_sha256: study.study_sha256.clone(),
            result,
            reason: reason.chars().take(64).collect(),
            prev_sha256: self.tip.clone(),
        };
        let bytes = serde_json::to_vec(&record)?;
        write_new(&self.dir, &name(record.seq), &bytes)?;
        self.tip = stutter_core::hash::sha256_hex(&bytes);
        self.next_seq += 1;
        self.results
            .insert(record.study_sha256, (record.result, record.reason));
        Ok(())
    }

    fn results_only(&self) -> BTreeMap<String, StudyResult> {
        self.results
            .iter()
            .map(|(digest, (result, _))| (digest.clone(), *result))
            .collect()
    }

    /// Decide everything that cannot run (budget, failed prerequisites, a blocker the caller
    /// names for a study) and return the next study that can, or `None` when the funnel is done.
    /// `blocker` returns a reason when a ready study must not start (for example no way to apply
    /// its graphics profile); it is recorded as `Blocked`, never run and never dropped.
    pub fn next_runnable(
        &mut self,
        remaining_attempts: u32,
        blocker: &dyn Fn(&ChildStudy) -> Option<&'static str>,
    ) -> anyhow::Result<Option<ChildStudy>> {
        loop {
            let plan = self.plan.clone();
            match next_study(&plan, &self.results_only(), remaining_attempts) {
                NextStudy::Done => return Ok(None),
                NextStudy::DeferBudget(study) => {
                    self.record(study, StudyResult::DeferredBudget, "DeferredBudget")?;
                }
                NextStudy::Blocked(study) => {
                    self.record(study, StudyResult::Blocked, "PredecessorNotQualified")?;
                }
                NextStudy::Run(study) => match blocker(study) {
                    Some(reason) => self.record(study, StudyResult::Blocked, reason)?,
                    None => return Ok(Some(study.clone())),
                },
            }
        }
    }

    /// The plan with no decision recorded (a campaign that has no ledger).
    pub fn undecided(plan: Vec<ChildStudy>) -> Vec<Disposition> {
        plan.into_iter()
            .map(|study| Disposition {
                study_sha256: study.study_sha256,
                profile: study.profile,
                stage: study.stage,
                result: None,
                reason: String::new(),
            })
            .collect()
    }

    /// Every planned study in funnel order with its recorded outcome.
    pub fn dispositions(&self) -> Vec<Disposition> {
        self.plan
            .iter()
            .map(|study| {
                let decided = self.results.get(&study.study_sha256);
                Disposition {
                    study_sha256: study.study_sha256.clone(),
                    profile: study.profile,
                    stage: study.stage,
                    result: decided.map(|(result, _)| *result),
                    reason: decided
                        .map(|(_, reason)| reason.clone())
                        .unwrap_or_default(),
                }
            })
            .collect()
    }
}

/// Why a ready study cannot start in this build: its graphics profile cannot be applied to an
/// isolated prefix, and the live action child has no authorization bridge yet. Both are recorded,
/// never silently skipped.
pub fn unexecutable_reason(study: &ChildStudy) -> Option<&'static str> {
    Some(match study.profile {
        Profile::Cpu | Profile::Gpu => "BlockedProfileApplier",
        Profile::Long => "BlockedActionChild",
    })
}

fn open_ledger(
    campaign: &File,
    uid: u32,
    intent: &super::ResearchIntentV1,
    create: bool,
) -> anyhow::Result<ChildLedger> {
    use std::os::fd::AsFd;
    if create {
        match crate::syscall::mkdir_private_at(campaign.as_fd(), "child-studies") {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }
    let dir = File::from(crate::syscall::openat_dir_nofollow(
        campaign.as_fd(),
        "child-studies",
    )?);
    ChildLedger::open(dir, uid, super::child_study::plan(intent)?)
}

/// At campaign end: every planned child study that the flat schedule did not execute is decided
/// and recorded (restart-safe; already-recorded decisions are kept).
pub fn settle_unexecuted(
    campaign: &File,
    uid: u32,
    intent: &super::ResearchIntentV1,
) -> anyhow::Result<()> {
    let mut ledger = open_ledger(campaign, uid, intent, true)?;
    ledger.next_runnable(intent.max_attempts, &unexecutable_reason)?;
    Ok(())
}

/// Read-only: the plan with whatever has been decided. A campaign without a ledger reports every
/// study undecided.
pub fn read_dispositions(
    campaign: &File,
    uid: u32,
    intent: &super::ResearchIntentV1,
) -> anyhow::Result<Vec<Disposition>> {
    match open_ledger(campaign, uid, intent, false) {
        Ok(ledger) => Ok(ledger.dispositions()),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound) =>
        {
            Ok(ChildLedger::undecided(super::child_study::plan(intent)?))
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
#[path = "child_ledger_tests.rs"]
mod tests;
