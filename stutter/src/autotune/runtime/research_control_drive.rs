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
    campaign: File,
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
        let evidence = create_dir(&campaign, "evidence")?;
        crate::autotune::research::report_store::write_new(
            &campaign,
            "intent.json",
            &serde_json::to_vec(intent)?,
        )?;
        let schedule = build_schedule(intent)?;
        let len = schedule.len();
        let journal = ResearchJournal::open(journal_dir.into(), intent.uid, schedule)?;
        self.driver = Some(DriverState {
            driver: Driver::new(journal, intent.clone(), dispatch, evidence, len),
            host: factory(),
            campaign,
        });
        Ok(())
    }

    pub(in crate::autotune::runtime) fn drive_due(&self) -> bool {
        self.storage_blocker != Some("UnsafeToResume")
            && self.driver.is_some()
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
                if let Err(settlement) = state.host.abort() {
                    log::error!("research process settlement is unverified: {settlement:#}");
                }
                // A failed tick may have left a started marker or torn evidence even if the host
                // stopped. Keep the lease and mutation slot until independent recovery proves it.
                self.storage_blocker = Some("UnsafeToResume");
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
        if matches!(step, Step::Done) && ok && !self.persist_report() {
            self.settle(now, ResearchPhase::Blocked, "ReportPersistenceFailed");
            return;
        }
        if matches!(step, Step::Done) && ok {
            self.settle_terminal(ResearchPhase::Completed, "Completed");
        } else if !ok {
            if let Some(state) = self.driver.as_mut()
                && let Err(error) = state.host.abort()
            {
                log::error!("research process settlement is unverified: {error:#}");
            }
            self.storage_blocker = Some("UnsafeToResume");
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

    /// Settle an in-process attempt before the controller and worker shut down. An unsealed or
    /// previously uncertain attempt remains recoverable only through the existing safety gate.
    pub(in crate::autotune::runtime) fn shutdown_driver(&mut self, now: u64) -> anyhow::Result<()> {
        let Some(state) = self.driver.as_mut() else {
            return Ok(());
        };
        if self.storage_blocker == Some("UnsafeToResume") {
            state.host.abort()?;
            anyhow::bail!("research attempt requires independent recovery");
        }
        state.driver.cancel();
        match state.driver.tick(state.host.as_mut(), now)? {
            Step::Sealed(_) | Step::Cancelled => {}
            _ => anyhow::bail!("research shutdown did not settle the attempt"),
        }
        self.settle(now, ResearchPhase::Aborted, "ControllerShutdown");
        anyhow::ensure!(
            self.terminal
                .as_ref()
                .is_some_and(|terminal| terminal.phase == ResearchPhase::Aborted),
            "research lease shutdown was not persisted"
        );
        Ok(())
    }

    fn settle(&mut self, now: u64, terminal: ResearchPhase, code: &'static str) {
        if let Some(active) = self.active.as_mut() {
            let phase = active.lease.phase();
            if !matches!(
                phase,
                ResearchPhase::Completed | ResearchPhase::Blocked | ResearchPhase::Aborted
            ) {
                let restoring = phase == ResearchPhase::Restoring
                    || active.lease.advance(ResearchPhase::Restoring, now).is_ok();
                if !restoring || active.lease.advance(terminal, now).is_err() {
                    self.storage_blocker = Some("UnsafeToResume");
                    return;
                }
            }
            if active.lease.phase() != terminal {
                self.storage_blocker = Some("UnsafeToResume");
                return;
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
    fn abort(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
}

impl ResearchCoordinator {
    /// Write `report.json` for a finished schedule. Called while the driver still owns the journal.
    fn persist_report(&mut self) -> bool {
        let Some(state) = self.driver.as_ref() else {
            return false;
        };
        let result = state.driver.finalize().and_then(|report| {
            crate::autotune::research::report_store::write_report(&state.campaign, &report)
        });
        if let Err(error) = &result {
            log::error!("research report could not be persisted: {error:#}");
        }
        result.is_ok()
    }

    /// Locate the campaign's private study directory for Report/Verify.
    pub(super) fn open_campaign_study_dir(&self) -> Option<File> {
        let digest = self
            .active
            .as_ref()
            .map(|active| active.intent_sha256.as_str())
            .or_else(|| self.terminal.as_ref().map(|t| t.intent_sha256.as_str()))?;
        let study =
            File::from(crate::syscall::open_dir_no_symlinks(self.study_dir.as_deref()?).ok()?);
        use std::os::fd::AsFd;
        let dir = crate::syscall::openat_dir_nofollow(study.as_fd(), digest.get(..16)?).ok()?;
        Some(File::from(dir))
    }

    pub(super) fn report_response(&self, verify: bool, uid: u32) -> String {
        let Some(dir) = self.open_campaign_study_dir() else {
            return super::response(
                "blocked",
                if verify {
                    "NoReportToVerify"
                } else {
                    "NoReport"
                },
            );
        };
        match crate::autotune::research::report_store::load_and_verify(&dir, uid) {
            Ok(report) if verify => {
                serde_json::json!({
                    "status": "blocked", "code": "IncompleteProvenance",
                    "reconstruction": "passed",
                    "intent_sha256": report.intent_sha256,
                })
                .to_string()
                    + "\n"
            }
            Ok(report) => {
                serde_json::json!({ "status": "ok", "code": "Report", "report": report })
                    .to_string()
                    + "\n"
            }
            Err(error) => {
                log::warn!("research report unavailable: {error:#}");
                super::response(
                    "blocked",
                    if verify {
                        "VerificationFailed"
                    } else {
                        "NoReport"
                    },
                )
            }
        }
    }
}
