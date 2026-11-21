//! The campaign's initialization phase: before any screen, every arm whose prefix is cold is
//! initialized in turn by `prefix_init_stage`, one supervised non-measuring launch each. The phase is
//! stateless between ticks (each tick reopens the arm's journal), so a restart resumes from what the
//! journal proves; the first tick after construction treats an unfinished launch as interrupted and
//! settles it instead of replaying it. An arm that is abandoned simply stays cold, and the profile hook
//! then blocks the studies that need it. Only an unprovable settlement ends the campaign.

use std::{fs::File, path::PathBuf};

use super::{
    ProtonSource,
    prefix_identity::{PrefixClass, observe},
    prefix_init::{InitStage, initialized},
    prefix_init_stage::{InitHost, InitProcess, InitStageRunner, InitStep},
};

/// One arm the phase may initialize.
#[derive(Clone, Debug)]
pub struct InitArm {
    pub source: ProtonSource,
    pub compat: PathBuf,
    pub settings: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PhaseStep {
    Pending,
    Finished,
    Unresolved(&'static str),
}

/// What happened to one arm, for the report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArmOutcome {
    pub source: ProtonSource,
    pub verified: bool,
    pub reason: Option<&'static str>,
}

/// Calls the controller's launch fence (the lease's launching phase) before the host launches.
struct Fenced<'a> {
    host: &'a mut dyn InitHost,
    fence: &'a mut dyn FnMut() -> anyhow::Result<()>,
}

impl InitHost for Fenced<'_> {
    fn launch(&mut self, arm: ProtonSource) -> Result<(), &'static str> {
        (self.fence)().map_err(|_| "LaunchFenceRefused")?;
        self.host.launch(arm)
    }
    fn process(&mut self) -> Result<InitProcess, &'static str> {
        self.host.process()
    }
    fn terminate(&mut self) -> Result<(), &'static str> {
        self.host.terminate()
    }
}

pub struct InitPhase {
    dir: File,
    uid: u32,
    arms: Vec<InitArm>,
    host: Box<dyn InitHost + Send>,
    /// Seconds of initialization the campaign still authorizes.
    authorized_seconds: u64,
    cursor: usize,
    recovered: bool,
    cancelled: bool,
    used_seconds: u64,
    outcomes: Vec<ArmOutcome>,
}

impl InitPhase {
    pub fn new(
        dir: File,
        uid: u32,
        arms: Vec<InitArm>,
        host: Box<dyn InitHost + Send>,
        authorized_seconds: u64,
    ) -> Self {
        Self {
            dir,
            uid,
            arms,
            host,
            authorized_seconds,
            cursor: 0,
            recovered: false,
            cancelled: false,
            used_seconds: 0,
            outcomes: Vec::new(),
        }
    }

    pub fn used_seconds(&self) -> u64 {
        self.used_seconds
    }

    pub fn outcomes(&self) -> &[ArmOutcome] {
        &self.outcomes
    }

    pub fn cancel(&mut self) {
        self.cancelled = true;
    }

    /// Controller shutdown: cancel whatever the phase launched and prove it gone. `Err` is unproven.
    pub fn shutdown(&mut self, now: u64) -> Result<(), &'static str> {
        self.cancelled = true;
        for _ in 0..=self.arms.len() {
            match self.tick(now, &mut || Ok(())) {
                PhaseStep::Unresolved(reason) => return Err(reason),
                PhaseStep::Finished => break,
                PhaseStep::Pending => {}
            }
        }
        Ok(())
    }

    /// Whether this arm has anything to initialize: a cold prefix, or a journal still in progress.
    fn needs(&self, arm: &InitArm) -> bool {
        match initialized(&self.dir, arm.source, &arm.compat, self.uid) {
            Ok(true) => false,
            Ok(false) => {
                matches!(observe(&arm.compat).class, PrefixClass::Cold) || {
                    super::prefix_init::InitJournal::open(&self.dir, arm.source, self.uid)
                        .ok()
                        .and_then(|journal| journal.stage())
                        .is_some_and(|stage| {
                            !matches!(stage, InitStage::Abandoned | InitStage::Verified)
                        })
                }
            }
            // A journal that does not read as a chain is never advanced.
            Err(_) => false,
        }
    }

    pub fn tick(&mut self, now: u64, fence: &mut dyn FnMut() -> anyhow::Result<()>) -> PhaseStep {
        loop {
            let Some(arm) = self.arms.get(self.cursor).cloned() else {
                return PhaseStep::Finished;
            };
            if !self.needs(&arm) {
                self.cursor += 1;
                continue;
            }
            let remaining = self.authorized_seconds.saturating_sub(self.used_seconds);
            let mut runner = match InitStageRunner::open(
                &self.dir,
                arm.source,
                &arm.compat,
                &arm.settings,
                self.uid,
                remaining,
            ) {
                Ok(runner) => runner,
                Err(_) => return PhaseStep::Unresolved("InitJournalUnreadable"),
            };
            let mut host = Fenced {
                host: self.host.as_mut(),
                fence,
            };
            let step = if self.cancelled {
                runner.cancel(&mut host, now)
            } else if !self.recovered {
                self.recovered = true;
                runner.recover(&mut host, now)
            } else {
                runner.tick(&mut host, now)
            };
            let launched = runner.launched_at();
            match step {
                InitStep::Waiting => return PhaseStep::Pending,
                InitStep::Unresolved(reason) => return PhaseStep::Unresolved(reason),
                InitStep::Verified | InitStep::Abandoned(_) => {
                    let (verified, reason) = match step {
                        InitStep::Abandoned(reason) => (false, Some(reason)),
                        _ => (true, None),
                    };
                    if let Some(at) = launched {
                        self.used_seconds += now.saturating_sub(at);
                    }
                    self.outcomes.push(ArmOutcome {
                        source: arm.source,
                        verified,
                        reason,
                    });
                    self.cursor += 1;
                    // Move straight on to the next arm in this tick only when nothing is running.
                    if self.cancelled {
                        return PhaseStep::Pending;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "init_phase_tests.rs"]
mod tests;
