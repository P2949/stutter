//! Resident campaign driver: a synchronous, host-abstracted step machine that turns a frozen
//! intent into durable, bounded, single-shot attempts. It never replays: an attempt's `started`
//! marker is durable before arming. A refusal, timeout, cancellation or host failure is sealed
//! only after the host confirms settlement; otherwise the started marker remains for recovery.

use std::fs::File;

use super::{
    AttemptOutcome, AttemptRecord, AttemptSlot, ResearchIntentV1, ResearchJournal,
    dispatch_context::{ArmSpec, DispatchContextV1, arm_attempt_in_root},
    report::AttemptEvidenceV1,
    report_store::{build_from_dirs, write_evidence},
    scheduler,
};

const ARM_LIFETIME_SECONDS: u64 = 120;

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the real Steam host reports these once launching is qualified"
    )
)]
#[derive(Clone, Debug, PartialEq)]
pub enum Progress {
    Running,
    Finished { evidence: AttemptEvidenceV1 },
    Failed { code: String },
}

/// Effects on the real world. Implementations must be idempotent for `abort`.
pub trait AttemptHost {
    /// True only in a qualified automatic safe window.
    fn safe_window(&mut self) -> bool;
    fn arm_spec(&mut self, slot: &AttemptSlot) -> anyhow::Result<ArmSpec>;
    /// Start the game through Steam after the attempt is durable and armed.
    fn launch(&mut self, slot: &AttemptSlot, context: &DispatchContextV1) -> anyhow::Result<()>;
    fn poll(&mut self) -> Progress;
    fn abort(&mut self) -> anyhow::Result<()>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    Waiting,
    Running(u32),
    Sealed(AttemptRecord),
    Done,
    Expired,
    Cancelled,
}

pub struct Driver {
    journal: ResearchJournal,
    intent: ResearchIntentV1,
    root: File,
    evidence: File,
    schedule_len: usize,
    max_attempts: usize,
    active: Option<(AttemptSlot, u64)>,
    cancel: bool,
}

/// Deterministic schedule: a pure function of the intent, so a verifier can regenerate it.
pub fn build_schedule(intent: &ResearchIntentV1) -> anyhow::Result<Vec<AttemptSlot>> {
    let digest = intent.digest()?;
    let arms: Vec<_> = intent.tools.iter().map(|tool| tool.source).collect();
    anyhow::ensure!(
        arms.len() >= 2,
        "a comparison needs at least two eligible tools"
    );
    let blocks = intent.max_attempts as usize / arms.len();
    anyhow::ensure!(blocks > 0, "max_attempts cannot hold one complete block");
    let seed = u64::from_str_radix(&digest[..16], 16)?;
    let slots = scheduler::balanced(arms.len(), blocks, seed)
        .map_err(|error| anyhow::anyhow!("invalid schedule: {error:?}"))?;
    let context_sha256 = stutter_core::hash::sha256_hex(
        format!(
            "{}:{}:{}",
            intent.source_sha256, intent.policy_sha256, intent.build_id
        )
        .as_bytes(),
    );
    Ok(slots
        .into_iter()
        .enumerate()
        .map(|(index, slot)| AttemptSlot {
            study_sha256: digest.clone(),
            context_sha256: context_sha256.clone(),
            index: index as u32,
            arm: arms[slot.arm],
        })
        .collect())
}

impl Driver {
    pub fn new(
        journal: ResearchJournal,
        intent: ResearchIntentV1,
        root: File,
        evidence: File,
        schedule_len: usize,
    ) -> Self {
        let max_attempts = intent.max_attempts as usize;
        Self {
            journal,
            intent,
            root,
            evidence,
            schedule_len,
            max_attempts,
            active: None,
            cancel: false,
        }
    }

    /// Build the final report from everything sealed so far.
    pub fn finalize(&self) -> anyhow::Result<super::report::ResearchReportV1> {
        build_from_dirs(&self.intent, &self.journal, &self.evidence)
    }

    pub fn cancel(&mut self) {
        self.cancel = true;
    }

    fn seal(&mut self, slot: AttemptSlot, outcome: AttemptOutcome) -> anyhow::Result<Step> {
        let record = AttemptRecord { slot, outcome };
        self.journal.seal(record.clone())?;
        self.active = None;
        Ok(Step::Sealed(record))
    }

    pub fn tick(&mut self, host: &mut dyn AttemptHost, now: u64) -> anyhow::Result<Step> {
        if let Some((slot, started)) = self.active.clone() {
            let failure = if self.cancel {
                Some("Cancelled")
            } else if now >= self.intent.deadline_unix_seconds {
                Some("CampaignDeadline")
            } else if now.saturating_sub(started) >= u64::from(self.intent.max_attempt_seconds) {
                Some("AttemptTimeout")
            } else {
                None
            };
            if let Some(code) = failure {
                host.abort()?;
                return self.seal(slot, AttemptOutcome::Failed { code: code.into() });
            }
            return match host.poll() {
                Progress::Running => Ok(Step::Running(slot.index)),
                Progress::Finished { evidence } => {
                    match write_evidence(&self.evidence, &slot, &evidence) {
                        Ok(evidence_sha256) => {
                            self.seal(slot, AttemptOutcome::Completed { evidence_sha256 })
                        }
                        Err(_) => {
                            host.abort()?;
                            self.seal(
                                slot,
                                AttemptOutcome::Failed {
                                    code: "EvidenceRejected".into(),
                                },
                            )
                        }
                    }
                }
                Progress::Failed { code } => {
                    host.abort()?;
                    self.seal(slot, AttemptOutcome::Failed { code })
                }
            };
        }
        if self.cancel {
            return Ok(Step::Cancelled);
        }
        if now >= self.intent.deadline_unix_seconds {
            return Ok(Step::Expired);
        }
        let done = self.journal.verify()?.len();
        if done >= self.schedule_len || done >= self.max_attempts {
            return Ok(Step::Done);
        }
        if !host.safe_window() {
            return Ok(Step::Waiting);
        }
        let slot = self.journal.start()?;
        self.active = Some((slot.clone(), now));
        let launched = host
            .arm_spec(&slot)
            .and_then(|spec| {
                arm_attempt_in_root(
                    &self.root,
                    &self.intent,
                    &slot,
                    spec,
                    (now, ARM_LIFETIME_SECONDS),
                )
            })
            .and_then(|context| host.launch(&slot, &context));
        match launched {
            Ok(()) => Ok(Step::Running(slot.index)),
            Err(_) => {
                host.abort()?;
                self.seal(
                    slot,
                    AttemptOutcome::Failed {
                        code: "LaunchRefused".into(),
                    },
                )
            }
        }
    }
}

#[cfg(test)]
#[path = "driver_tests.rs"]
mod tests;
