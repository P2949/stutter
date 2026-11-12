//! Resident attempt driving. The coordinator owns one `Driver` per admitted campaign and advances
//! the durable lease phases only from what the driver observed. Without a host factory (tests,
//! unqualified builds) no attempt is ever scheduled.
use std::fs::File;

use super::{ActiveCampaign, ResearchCoordinator, TerminalCampaign};
use crate::autotune::research::{
    ResearchIntentV1, ResearchJournal, ResearchPhase,
    driver::{AttemptHost, Driver, Step, build_schedule},
};

pub(in crate::autotune::runtime) type HostFactory =
    Box<dyn Fn() -> Box<dyn AttemptHost + Send> + Send>;

pub(super) struct DriverState {
    driver: Driver,
    host: Box<dyn AttemptHost + Send>,
}

fn create_dir(parent: &File, name: &str) -> anyhow::Result<File> {
    use std::os::fd::AsFd;
    crate::syscall::mkdir_private_at(parent.as_fd(), name)?;
    Ok(File::from(crate::syscall::openat_dir_nofollow(
        parent.as_fd(),
        name,
    )?))
}

impl ResearchCoordinator {
    pub(in crate::autotune::runtime) fn set_host_factory(&mut self, factory: Option<HostFactory>) {
        self.host_factory = factory;
    }

    /// Create the campaign's private journal and dispatch directories and its driver. A no-op
    /// when no host factory is configured.
    pub(super) fn attach_driver(
        &mut self,
        intent: &ResearchIntentV1,
        digest: &str,
    ) -> anyhow::Result<()> {
        let Some(factory) = self.host_factory.as_ref() else {
            return Ok(());
        };
        let study = self
            .study_dir
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("no study directory"))?;
        let study = File::from(crate::syscall::open_dir_no_symlinks(study)?);
        let campaign = create_dir(&study, &digest[..16])?;
        let journal_dir = create_dir(&campaign, "journal")?;
        let dispatch = create_dir(&campaign, "dispatch")?;
        let schedule = build_schedule(intent)?;
        let len = schedule.len();
        let journal = ResearchJournal::open(journal_dir.into(), intent.uid, schedule)?;
        self.driver = Some(DriverState {
            driver: Driver::new(journal, intent.clone(), dispatch, len),
            host: factory(),
        });
        Ok(())
    }

    pub(in crate::autotune::runtime) fn drive_due(&self) -> bool {
        self.driver.is_some()
            && self.active.as_ref().is_some_and(|active| {
                matches!(
                    active.lease.phase(),
                    ResearchPhase::AwaitSafeWindow
                        | ResearchPhase::Launching
                        | ResearchPhase::Attesting
                        | ResearchPhase::Warmup
                        | ResearchPhase::Measuring
                )
            })
    }

    pub(in crate::autotune::runtime) fn drive_attempts(&mut self, now: u64) {
        if !self.drive_due() {
            return;
        }
        let Some(state) = self.driver.as_mut() else {
            return;
        };
        let step = match state.driver.tick(state.host.as_mut(), now) {
            Ok(step) => step,
            Err(error) => {
                log::error!("research driver failed: {error:#}");
                state.host.abort();
                self.settle(now, ResearchPhase::Blocked, "DriverFailed");
                return;
            }
        };
        let Some(active) = self.active.as_mut() else {
            return;
        };
        let advance = |active: &mut ActiveCampaign, phases: &[ResearchPhase]| {
            phases
                .iter()
                .all(|phase| active.lease.advance(*phase, now).is_ok())
        };
        let ok = match step {
            Step::Waiting => true,
            Step::Running(_) => {
                active.lease.phase() != ResearchPhase::AwaitSafeWindow
                    || advance(active, &[ResearchPhase::Launching])
            }
            Step::Sealed(_) => {
                use ResearchPhase::*;
                let phase = active.lease.phase();
                let mut path = vec![];
                if phase == AwaitSafeWindow {
                    path.push(Launching);
                }
                path.extend([Sealing, Analysing, NextArm, AwaitSafeWindow]);
                advance(active, &path)
            }
            Step::Done => advance(
                active,
                &[ResearchPhase::Restoring, ResearchPhase::Completed],
            ),
            Step::Expired => {
                self.on_deadline(now);
                true
            }
            Step::Cancelled => {
                self.settle(now, ResearchPhase::Aborted, "Cancelled");
                true
            }
        };
        if matches!(step, Step::Done) && ok {
            self.settle_terminal(ResearchPhase::Completed, "Completed");
        } else if !ok {
            self.settle(now, ResearchPhase::Blocked, "LeasePersistenceFailed");
        }
    }

    /// Request cancellation of a driven attempt; the next drive tick aborts and seals it.
    pub(super) fn request_driver_cancel(&mut self) -> bool {
        match self.driver.as_mut() {
            Some(state) => {
                state.driver.cancel();
                true
            }
            None => false,
        }
    }

    fn settle(&mut self, now: u64, terminal: ResearchPhase, code: &'static str) {
        if let Some(active) = self.active.as_mut() {
            let phase = active.lease.phase();
            if !matches!(
                phase,
                ResearchPhase::Completed | ResearchPhase::Blocked | ResearchPhase::Aborted
            ) {
                let _ = active.lease.advance(ResearchPhase::Restoring, now);
                let _ = active.lease.advance(terminal, now);
            }
        }
        self.settle_terminal(terminal, code);
    }

    fn settle_terminal(&mut self, phase: ResearchPhase, code: &'static str) {
        if let Some(active) = self.active.take() {
            self.terminal = Some(TerminalCampaign {
                phase,
                intent_sha256: active.intent_sha256,
                code,
            });
        }
        self.driver = None;
    }
}

/// Production host until the Steam launcher is physically qualified: it never reports a safe
/// window, so no attempt is ever journalled, armed or launched. The campaign waits to its deadline.
pub(in crate::autotune::runtime) struct UnqualifiedSteamHost;

impl AttemptHost for UnqualifiedSteamHost {
    fn safe_window(&mut self) -> bool {
        false
    }
    fn arm_spec(
        &mut self,
        _: &crate::autotune::research::AttemptSlot,
    ) -> anyhow::Result<crate::autotune::research::dispatch_context::ArmSpec> {
        anyhow::bail!("BlockedSteamAutomation")
    }
    fn launch(
        &mut self,
        _: &crate::autotune::research::AttemptSlot,
        _: &crate::autotune::research::dispatch_context::DispatchContextV1,
    ) -> anyhow::Result<()> {
        anyhow::bail!("BlockedSteamAutomation")
    }
    fn poll(&mut self) -> crate::autotune::research::driver::Progress {
        crate::autotune::research::driver::Progress::Failed {
            code: "BlockedSteamAutomation".into(),
        }
    }
    fn abort(&mut self) {}
}
