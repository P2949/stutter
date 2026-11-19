//! Durable, restart-safe record of the child-study funnel. Every decision about a planned study
//! (qualified, unqualified, failed, deferred for budget, blocked) is one immutable, hash-chained
//! record in the campaign directory, so a restarted controller resumes at the first study without a
//! record, a tampered or reordered ledger is refused, and no study is silently dropped or run twice.

use std::{collections::BTreeMap, fs::File};

use serde::{Deserialize, Serialize};

use super::{
    child_study::{ChildStudy, NextStudy, Profile, Stage, StudyResult, next_study},
    pilot_rule::FrozenConfirmationRule,
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
    /// Attempts this study consumed (zero for a decision made without running it).
    #[serde(default)]
    attempts_used: u32,
    /// The confirmation rule a qualified pilot froze (digest-bound; absent for other studies).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rule: Option<FrozenConfirmationRule>,
    /// SHA-256 of the previous record's bytes (all zeros for the first).
    prev_sha256: String,
}

/// Verify plan order and prerequisites independently of a record's claimed result.
fn validate_next(
    plan: &[ChildStudy],
    results: &BTreeMap<String, (StudyResult, String)>,
    record: &Record,
) -> anyhow::Result<()> {
    let study = plan
        .get(results.len())
        .ok_or_else(|| anyhow::anyhow!("ledger has more decisions than its plan"))?;
    anyhow::ensure!(
        study.study_sha256 == record.study_sha256,
        "ledger decision is out of plan order"
    );
    let prior = results
        .iter()
        .map(|(digest, (result, _))| (digest.clone(), *result))
        .collect();
    let ready = super::child_study::ready_to_start(study, &prior).is_ok();
    if record.result == StudyResult::Blocked {
        anyhow::ensure!(!record.reason.is_empty(), "blocked decision has no reason");
        anyhow::ensure!(
            (record.reason == "PredecessorNotQualified") != ready,
            "blocked decision contradicts its prerequisites"
        );
    } else {
        anyhow::ensure!(ready, "ledger decision has an unqualified prerequisite");
    }
    if record.result == StudyResult::Qualified {
        anyhow::ensure!(
            record.attempts_used >= study.min_attempts,
            "qualified study has insufficient attempts"
        );
        if matches!(study.stage, Stage::Pilot) {
            anyhow::ensure!(record.rule.is_some(), "qualified pilot has no frozen rule");
        }
    }
    if matches!(
        record.result,
        StudyResult::Blocked | StudyResult::DeferredBudget
    ) {
        anyhow::ensure!(
            record.attempts_used == 0,
            "unexecuted decision claims attempts"
        );
    }
    if let Some(rule) = &record.rule {
        anyhow::ensure!(
            matches!(study.stage, super::child_study::Stage::Pilot)
                && record.result == StudyResult::Qualified
                && rule.is_consistent(),
            "only a qualified pilot may record a rule"
        );
    }
    Ok(())
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
    attempts: BTreeMap<String, u32>,
    rules: BTreeMap<String, FrozenConfirmationRule>,
    tip: String,
    next_seq: u32,
    /// Attempts the recorded studies consumed, so the budget survives a restart.
    spent: u32,
    max_attempts: Option<u32>,
}

fn name(seq: u32) -> String {
    format!("{seq:04}.json")
}

impl ChildLedger {
    /// Open the ledger directory and verify every record: canonical names without gaps, a valid
    /// hash chain, only planned studies and at most one decision per study.
    #[cfg(test)]
    pub fn open(dir: File, uid: u32, plan: Vec<ChildStudy>) -> anyhow::Result<Self> {
        Self::open_with_limit(dir, uid, plan, None)
    }

    fn open_with_limit(
        dir: File,
        uid: u32,
        plan: Vec<ChildStudy>,
        max_attempts: Option<u32>,
    ) -> anyhow::Result<Self> {
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
            attempts: BTreeMap::new(),
            rules: BTreeMap::new(),
            tip: GENESIS.to_owned(),
            next_seq: 0,
            spent: 0,
            max_attempts,
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
            validate_next(&ledger.plan, &ledger.results, &record)?;
            ledger
                .attempts
                .insert(record.study_sha256.clone(), record.attempts_used);
            if let Some(rule) = record.rule.clone() {
                anyhow::ensure!(
                    record.result == StudyResult::Qualified && rule.is_consistent(),
                    "the ledger holds a rule that no qualified pilot could have frozen"
                );
                ledger.rules.insert(record.study_sha256.clone(), rule);
            }
            anyhow::ensure!(
                ledger
                    .results
                    .insert(record.study_sha256, (record.result, record.reason))
                    .is_none(),
                "a study has two decisions"
            );
            ledger.tip = stutter_core::hash::sha256_hex(&bytes);
            ledger.next_seq = seq + 1;
            ledger.spent = ledger.checked_spend(record.attempts_used)?;
        }
        Ok(ledger)
    }

    /// Record a decision made without running the study (it consumed no attempts).
    pub fn record(
        &mut self,
        study: &ChildStudy,
        result: StudyResult,
        reason: &str,
    ) -> anyhow::Result<()> {
        self.record_run(study, result, reason, 0)
    }

    /// Attempts the recorded studies have consumed so far.
    pub fn spent_attempts(&self) -> u32 {
        self.spent
    }

    pub(crate) fn attempts_for(&self, study_sha256: &str) -> Option<u32> {
        self.attempts.get(study_sha256).copied()
    }

    fn checked_spend(&self, attempts: u32) -> anyhow::Result<u32> {
        let total = self
            .spent
            .checked_add(attempts)
            .ok_or_else(|| anyhow::anyhow!("ledger attempt count overflow"))?;
        anyhow::ensure!(
            self.max_attempts.is_none_or(|limit| total <= limit),
            "ledger exceeds authorized attempts"
        );
        Ok(total)
    }

    /// Record one decision and the attempts the study consumed. A study decided already, or not in
    /// the plan, is refused.
    pub fn record_run(
        &mut self,
        study: &ChildStudy,
        result: StudyResult,
        reason: &str,
        attempts_used: u32,
    ) -> anyhow::Result<()> {
        self.record_run_with_rule(study, result, reason, attempts_used, None)
    }

    /// The rule a qualified pilot froze, once recorded.
    pub fn rule_of(&self, study_sha256: &str) -> Option<&FrozenConfirmationRule> {
        self.rules.get(study_sha256)
    }

    /// As `record_run`, also recording the confirmation rule a qualified pilot froze.
    pub fn record_run_with_rule(
        &mut self,
        study: &ChildStudy,
        result: StudyResult,
        reason: &str,
        attempts_used: u32,
        rule: Option<FrozenConfirmationRule>,
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
            attempts_used,
            rule: rule.clone(),
            prev_sha256: self.tip.clone(),
        };
        validate_next(&self.plan, &self.results, &record)?;
        let spent = self.checked_spend(attempts_used)?;
        let bytes = serde_json::to_vec(&record)?;
        write_new(&self.dir, &name(record.seq), &bytes)?;
        self.tip = stutter_core::hash::sha256_hex(&bytes);
        self.next_seq += 1;
        self.spent = spent;
        self.attempts
            .insert(record.study_sha256.clone(), attempts_used);
        if let Some(rule) = rule {
            self.rules.insert(study.study_sha256.clone(), rule);
        }
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

pub(crate) fn open_ledger(
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
    ChildLedger::open_with_limit(
        dir,
        uid,
        super::child_study::plan(intent)?,
        Some(intent.max_attempts),
    )
}

/// Open (creating it if needed) the campaign's child-study ledger over its frozen funnel plan.
pub fn open_for_campaign(
    campaign: &File,
    uid: u32,
    intent: &super::ResearchIntentV1,
) -> anyhow::Result<ChildLedger> {
    open_ledger(campaign, uid, intent, true)
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

/// Read-only: the rule each qualified pilot froze, by the pilot's study digest (none without a ledger).
pub fn read_rules(
    campaign: &File,
    uid: u32,
    intent: &super::ResearchIntentV1,
) -> anyhow::Result<BTreeMap<String, FrozenConfirmationRule>> {
    match open_ledger(campaign, uid, intent, false) {
        Ok(ledger) => Ok(ledger.rules),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound) =>
        {
            Ok(BTreeMap::new())
        }
        Err(error) => Err(error),
    }
}

/// The per-study counts asserted by the ledger, to be checked against child journals.
pub(crate) fn read_attempt_counts(
    campaign: &File,
    uid: u32,
    intent: &super::ResearchIntentV1,
) -> anyhow::Result<BTreeMap<String, u32>> {
    match open_ledger(campaign, uid, intent, false) {
        Ok(ledger) => Ok(ledger.attempts),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound) =>
        {
            Ok(BTreeMap::new())
        }
        Err(error) => Err(error),
    }
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
