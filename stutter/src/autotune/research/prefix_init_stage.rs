//! Runs one arm's prefix-initialization stage (PLAN section 6.1) as a resumable, incremental state
//! machine over the journalled contract in `prefix_init`. One supervised, non-measuring launch lets the
//! game create its prefix and settings; the stage then proves the launch settled and verifies the
//! prefix. Every host effect (launching, observing the launched process, terminating it) goes through
//! `InitHost`, so the production composition supplies the authenticated Steam launch and tests supply
//! a synthetic one. Nothing here measures, applies a profile or retries a launch: a launch found
//! unfinished after a restart is only ever settled, never replayed, and an arm that fails
//! to initialize is abandoned and stays blocked.

use std::{fs::File, path::Path};

use super::{
    ProtonSource,
    prefix_init::{InitJournal, InitStage, MAX_INIT_SECONDS},
};

/// What the supervised launch's process tree is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitProcess {
    Running,
    /// The launch exited and its whole process tree is independently proven gone.
    Settled,
}

/// The effects the stage needs; the production host is the authenticated Steam launcher.
pub trait InitHost {
    /// Start the one supervised, non-measuring launch for `arm`. Refusal is a reason code.
    fn launch(&mut self, arm: ProtonSource) -> Result<(), &'static str>;
    /// Whether the launched tree is still running or is proven gone. `Err` means unproven.
    fn process(&mut self) -> Result<InitProcess, &'static str>;
    /// Stop the launch and prove nothing it started remains. `Ok` only when proven.
    fn terminate(&mut self) -> Result<(), &'static str>;
}

/// What one tick of the stage decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InitStep {
    Waiting,
    /// The prefix is initialized and verified (or already was, by an earlier verified journal).
    Verified,
    /// Initialization was given up; the arm stays blocked. The reason is a stable code.
    Abandoned(&'static str),
    /// A launch could not be proven gone: nothing else may run in this campaign.
    Unresolved(&'static str),
}

pub struct InitStageRunner<'a> {
    journal: InitJournal<'a>,
    compat: &'a Path,
    settings: &'a Path,
    arm: ProtonSource,
    /// Seconds the campaign still authorizes for initialization; a launch needs the whole bound.
    authorized_seconds: u64,
}

impl<'a> InitStageRunner<'a> {
    pub fn open(
        dir: &'a File,
        arm: ProtonSource,
        compat: &'a Path,
        settings: &'a Path,
        uid: u32,
        authorized_seconds: u64,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            journal: InitJournal::open(dir, arm, uid)?,
            compat,
            settings,
            arm,
            authorized_seconds,
        })
    }

    fn record(&mut self, next: InitStage, now: u64) -> Result<(), &'static str> {
        self.journal
            .advance(next, self.compat, self.settings, now)
            .map_err(|_| "InitJournalWriteFailed")
    }

    /// Seconds a launched stage has been running, when it was launched.
    pub fn launched_at(&self) -> Option<u64> {
        self.journal.launched_at()
    }

    /// Advance the stage one step at time `now`.
    pub fn tick(&mut self, host: &mut dyn InitHost, now: u64) -> InitStep {
        match self.journal.stage() {
            Some(InitStage::Verified) => InitStep::Verified,
            Some(InitStage::Abandoned) => InitStep::Abandoned("InitAbandoned"),
            None => match self.record(InitStage::Planned, now) {
                Ok(()) => InitStep::Waiting,
                Err(reason) => InitStep::Unresolved(reason),
            },
            Some(InitStage::Planned) => {
                if self.authorized_seconds < MAX_INIT_SECONDS {
                    // Not enough authorized time to run it to its bound: give it up, never shorten it.
                    return self.abandon(now, "InitBudgetInsufficient");
                }
                if let Err(reason) = host.launch(self.arm) {
                    return self.abandon(now, reason);
                }
                match self.record(InitStage::Launched, now) {
                    Ok(()) => InitStep::Waiting,
                    Err(reason) => {
                        // A launch exists that the journal does not show: stop it before anything else.
                        let _ = host.terminate();
                        InitStep::Unresolved(reason)
                    }
                }
            }
            Some(InitStage::Launched) => self.watch(host, now),
            Some(InitStage::Settled) => self.verify(now),
        }
    }

    fn watch(&mut self, host: &mut dyn InitHost, now: u64) -> InitStep {
        let launched = self.launched_at().unwrap_or(0);
        let overdue = now.saturating_sub(launched) > MAX_INIT_SECONDS;
        let state = match host.process() {
            Ok(state) => state,
            Err(reason) => return InitStep::Unresolved(reason),
        };
        match (state, overdue) {
            (InitProcess::Running, false) => InitStep::Waiting,
            (InitProcess::Running, true) => match host.terminate() {
                Ok(()) => match self.record(InitStage::Settled, now) {
                    Ok(()) => self.abandon(now, "InitTimedOut"),
                    Err(reason) => InitStep::Unresolved(reason),
                },
                Err(reason) => InitStep::Unresolved(reason),
            },
            (InitProcess::Settled, _) => match self.record(InitStage::Settled, now) {
                Ok(()) => self.verify(now),
                Err(reason) => InitStep::Unresolved(reason),
            },
        }
    }

    fn verify(&mut self, now: u64) -> InitStep {
        match self.record(InitStage::Verified, now) {
            Ok(()) => InitStep::Verified,
            // The prefix does not hold what initialization creates (or it outlasted its bound).
            Err(_) => self.abandon(now, "PrefixNotInitialized"),
        }
    }

    fn abandon(&mut self, now: u64, reason: &'static str) -> InitStep {
        match self.record(InitStage::Abandoned, now) {
            Ok(()) => InitStep::Abandoned(reason),
            Err(code) => InitStep::Unresolved(code),
        }
    }

    /// Cancellation: stop whatever the stage launched and prove it gone before anything is recorded.
    pub fn cancel(&mut self, host: &mut dyn InitHost, now: u64) -> InitStep {
        match self.journal.stage() {
            Some(InitStage::Launched) => match host.terminate() {
                Ok(()) => match self.record(InitStage::Settled, now) {
                    Ok(()) => self.abandon(now, "InitCancelled"),
                    Err(reason) => InitStep::Unresolved(reason),
                },
                Err(reason) => InitStep::Unresolved(reason),
            },
            Some(InitStage::Verified) => InitStep::Verified,
            Some(InitStage::Abandoned) => InitStep::Abandoned("InitAbandoned"),
            None => match self.record(InitStage::Planned, now) {
                Ok(()) => self.abandon(now, "InitCancelled"),
                Err(reason) => InitStep::Unresolved(reason),
            },
            _ => self.abandon(now, "InitCancelled"),
        }
    }

    /// After a restart: a launch the journal shows as unfinished is settled, never replayed.
    pub fn recover(&mut self, host: &mut dyn InitHost, now: u64) -> InitStep {
        if self.journal.stage() == Some(InitStage::Launched) {
            return match host.terminate() {
                Ok(()) => match self.record(InitStage::Settled, now) {
                    Ok(()) => self.abandon(now, "InitInterrupted"),
                    Err(reason) => InitStep::Unresolved(reason),
                },
                Err(reason) => InitStep::Unresolved(reason),
            };
        }
        self.tick(host, now)
    }
}

#[cfg(test)]
#[path = "prefix_init_stage_tests.rs"]
mod tests;
