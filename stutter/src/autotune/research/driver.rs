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
    /// Latest independently observed idle timestamp, or None when occupied or unknown.
    fn observe_desktop_idle(&mut self, _idle_at: Option<std::time::Instant>) {}
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
    ActiveBudgetExhausted,
    Cancelled,
}

pub struct Driver {
    journal: ResearchJournal,
    intent: ResearchIntentV1,
    root: File,
    evidence: File,
    schedule_len: usize,
    max_attempts: usize,
    active: Option<(AttemptSlot, u64, std::time::Instant)>,
    dispatch_may_exist: bool,
    active_seconds: u64,
    last_tick_seconds: Option<u64>,
    fresh_journal_checked: bool,
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
            dispatch_may_exist: false,
            active_seconds: 0,
            last_tick_seconds: None,
            fresh_journal_checked: false,
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

    fn seal(
        &mut self,
        slot: AttemptSlot,
        outcome: AttemptOutcome,
        now: u64,
    ) -> anyhow::Result<Step> {
        use std::os::fd::AsFd;
        match crate::syscall::openat_dir_nofollow(self.root.as_fd(), &format!("{:04}", slot.index))
        {
            Ok(dir) => super::launch_ticket::retire_unclaimed_tickets(dir, self.intent.uid)?,
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound && !self.dispatch_may_exist => {}
            Err(error) => return Err(error.into()),
        }
        let record = AttemptRecord { slot, outcome };
        self.journal.seal(record.clone())?;
        if let Some((_, started, monotonic_start)) = &self.active {
            // Timestamps are whole seconds. Charge the partial boundary second so rounding
            // cannot expand the authorized active budget.
            self.active_seconds = self.active_seconds.saturating_add(
                now.saturating_sub(*started)
                    .max(monotonic_start.elapsed().as_secs())
                    .saturating_add(1),
            );
        }
        self.active = None;
        self.dispatch_may_exist = false;
        Ok(Step::Sealed(record))
    }

    pub fn tick(&mut self, host: &mut dyn AttemptHost, now: u64) -> anyhow::Result<Step> {
        let tick_started = std::time::Instant::now();
        anyhow::ensure!(
            self.last_tick_seconds.is_none_or(|last| now >= last),
            "campaign clock moved backward"
        );
        self.last_tick_seconds = Some(now);
        if !self.fresh_journal_checked {
            anyhow::ensure!(
                self.journal.verify()?.is_empty(),
                "UnsafeToResume: active-time budget requires independent recovery"
            );
            self.fresh_journal_checked = true;
        }
        if let Some((slot, started, monotonic_start)) = self.active.clone() {
            let failure = if self.cancel {
                Some("Cancelled")
            } else if now >= self.intent.deadline_unix_seconds {
                Some("CampaignDeadline")
            } else if self.active_seconds.saturating_add(
                now.saturating_sub(started)
                    .max(monotonic_start.elapsed().as_secs())
                    .saturating_add(1),
            ) >= u64::from(self.intent.max_active_seconds)
            {
                Some("ActiveBudgetExceeded")
            } else if now.saturating_sub(started) >= u64::from(self.intent.max_attempt_seconds) {
                Some("AttemptTimeout")
            } else {
                None
            };
            if let Some(code) = failure {
                host.abort()?;
                return self.seal(slot, AttemptOutcome::Failed { code: code.into() }, now);
            }
            let progress = host.poll();
            if now
                .saturating_add(tick_started.elapsed().as_secs())
                .saturating_add(1)
                >= self.intent.deadline_unix_seconds
            {
                host.abort()?;
                return self.seal(
                    slot,
                    AttemptOutcome::Failed {
                        code: "CampaignDeadline".into(),
                    },
                    now,
                );
            }
            if self.active_seconds.saturating_add(
                now.saturating_sub(started)
                    .max(monotonic_start.elapsed().as_secs())
                    .saturating_add(1),
            ) >= u64::from(self.intent.max_active_seconds)
            {
                host.abort()?;
                return self.seal(
                    slot,
                    AttemptOutcome::Failed {
                        code: "ActiveBudgetExceeded".into(),
                    },
                    now,
                );
            }
            if now
                .saturating_sub(started)
                .max(monotonic_start.elapsed().as_secs())
                >= u64::from(self.intent.max_attempt_seconds)
            {
                host.abort()?;
                return self.seal(
                    slot,
                    AttemptOutcome::Failed {
                        code: "AttemptTimeout".into(),
                    },
                    now,
                );
            }
            return match progress {
                Progress::Running => Ok(Step::Running(slot.index)),
                Progress::Finished { evidence } => {
                    match evidence
                        .validate_for(&slot, &self.intent)
                        .and_then(|()| write_evidence(&self.evidence, &slot, &evidence))
                    {
                        Ok(evidence_sha256) => {
                            self.seal(slot, AttemptOutcome::Completed { evidence_sha256 }, now)
                        }
                        Err(_) => {
                            host.abort()?;
                            self.seal(
                                slot,
                                AttemptOutcome::Failed {
                                    code: "EvidenceRejected".into(),
                                },
                                now,
                            )
                        }
                    }
                }
                Progress::Failed { code } => {
                    host.abort()?;
                    self.seal(slot, AttemptOutcome::Failed { code }, now)
                }
            };
        }
        if self.cancel {
            return Ok(Step::Cancelled);
        }
        if now >= self.intent.deadline_unix_seconds {
            return Ok(Step::Expired);
        }
        if self.active_seconds >= u64::from(self.intent.max_active_seconds) {
            return Ok(Step::ActiveBudgetExhausted);
        }
        let done = self.journal.verify()?.len();
        if done >= self.schedule_len || done >= self.max_attempts {
            return Ok(Step::Done);
        }
        if !host.safe_window() {
            return Ok(Step::Waiting);
        }
        let slot = self.journal.start()?;
        let monotonic_start = std::time::Instant::now();
        self.active = Some((slot.clone(), now, monotonic_start));
        let mut authorization_expired = false;
        let mut arm_failed = false;
        let launched = host
            .arm_spec(&slot)
            .and_then(|spec| {
                // Even a failed arm may have created a directory or a partial ticket.
                self.dispatch_may_exist = true;
                let result = arm_attempt_in_root(
                    &self.root,
                    &self.intent,
                    &slot,
                    spec,
                    (
                        now,
                        ARM_LIFETIME_SECONDS.min(self.intent.deadline_unix_seconds - now),
                    ),
                );
                arm_failed = result.is_err();
                result
            })
            .and_then(|context| {
                // Source discovery and arming can take long enough to exhaust the authorization
                // after the tick's wall-clock sample. Never start a launcher using stale time.
                let elapsed = monotonic_start.elapsed().as_secs();
                authorization_expired = now
                    .saturating_add(tick_started.elapsed().as_secs())
                    .saturating_add(1)
                    >= self.intent.deadline_unix_seconds
                    || elapsed >= u64::from(self.intent.max_attempt_seconds)
                    || self
                        .active_seconds
                        .saturating_add(elapsed)
                        .saturating_add(1)
                        >= u64::from(self.intent.max_active_seconds);
                anyhow::ensure!(
                    !authorization_expired,
                    "attempt authorization expired before launch"
                );
                host.launch(&slot, &context)
            });
        match launched {
            Ok(()) => Ok(Step::Running(slot.index)),
            Err(_) => {
                host.abort()?;
                // Arming can fail before or after creating the directory. Until its identity
                // is established by recovery, do not alter or seal potentially foreign state.
                anyhow::ensure!(!arm_failed, "attempt arming needs independent recovery");
                self.seal(
                    slot,
                    AttemptOutcome::Failed {
                        code: if authorization_expired {
                            "AuthorizationExpiredBeforeLaunch"
                        } else {
                            "LaunchRefused"
                        }
                        .into(),
                    },
                    now,
                )
            }
        }
    }
}

#[cfg(test)]
#[path = "driver_tests.rs"]
mod tests;
