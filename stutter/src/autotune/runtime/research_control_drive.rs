//! Resident attempt driving. The coordinator owns one `Driver` per admitted campaign and advances
//! the durable lease phases only from what the driver observed. Without a host factory (tests,
//! unqualified builds) no attempt is ever scheduled.
use std::{fs::File, path::PathBuf};

use super::{ActiveCampaign, ResearchCoordinator, TerminalCampaign};
use crate::{
    autotune::research::{
        ResearchIntentV1, ResearchJournal, ResearchPhase,
        driver::{
            AttemptHost, ClaimSettlementVerifier, Driver, LaunchUnresolved, Step, build_schedule,
        },
        funnel_driver::FunnelDriver,
        settlement::ProductionSettlementVerifier,
    },
    daemon::privilege::UnixSocketPrivilegedActionService,
};

/// Both flat and child drivers use the same independent process and worker witnesses.
pub(super) fn production_claim_verifier(socket: PathBuf) -> Box<dyn ClaimSettlementVerifier> {
    Box::new(ProductionSettlementVerifier::new(
        "/proc".into(),
        Some(Box::new(move || {
            UnixSocketPrivilegedActionService::new(socket.clone()).outstanding_tokens()
        })),
    ))
}

/// Builds the attempt host from the frozen intent. Failure means the host cannot be qualified, and
/// nothing durable has been created yet.
pub(in crate::autotune::runtime) type HostFactory =
    Box<dyn Fn(&ResearchIntentV1) -> anyhow::Result<Box<dyn AttemptHost + Send>> + Send>;

pub(super) struct DriverState {
    driver: Driver,
    host: Box<dyn AttemptHost + Send>,
    campaign: File,
    /// The child-study funnel, when the campaign runs in that mode. The flat `driver` is then an idle
    /// shell (never ticked) that only finalizes the campaign's own report, and `host` is a stub: the
    /// real host lives inside the funnel.
    funnel: Option<FunnelDriver>,
}

impl DriverState {
    /// Stop whatever the campaign owns (the flat host, or the funnel's shared host).
    fn abort_host(&mut self) -> anyhow::Result<()> {
        match self.funnel.as_mut() {
            // Settle, restore and prove the graphics profile; unproven is an error.
            Some(funnel) => funnel
                .shutdown()
                .map_err(|reason| anyhow::anyhow!("funnel settlement unproven: {reason}")),
            None => self.host.abort(),
        }
    }
}

/// Builds the child-study funnel from the frozen intent, the campaign directory and the host. It
/// exists only when the funnel mode is explicitly enabled; without it every campaign runs the flat
/// schedule exactly as before.
pub(in crate::autotune::runtime) type FunnelBuilder = Box<
    dyn Fn(&ResearchIntentV1, &File, Box<dyn AttemptHost + Send>) -> anyhow::Result<FunnelDriver>
        + Send,
>;

fn create_dir(parent: &File, name: &str) -> anyhow::Result<File> {
    use std::os::fd::AsFd;
    crate::syscall::mkdir_private_at(parent.as_fd(), name)?;
    Ok(File::from(crate::syscall::openat_dir_nofollow(
        parent.as_fd(),
        name,
    )?))
}

impl ResearchCoordinator {
    /// Worker socket used only for the read-only outstanding-token witness at claim settlement.
    pub(in crate::autotune::runtime) fn set_settlement_socket(&mut self, socket: Option<PathBuf>) {
        self.settlement_socket = socket;
    }

    pub(in crate::autotune::runtime) fn set_host_factory(&mut self, factory: Option<HostFactory>) {
        self.host_factory = factory;
    }

    /// Enable (or disable) the child-study funnel mode. Disabled is the default and keeps the flat
    /// schedule.
    pub(in crate::autotune::runtime) fn set_funnel_builder(
        &mut self,
        builder: Option<FunnelBuilder>,
    ) {
        self.funnel_builder = builder;
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
        // Qualify and build the host before any directory, journal or intent file exists.
        let host = factory(intent)?;
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
        let mut driver = Driver::new(journal, intent.clone(), dispatch, evidence, len);
        if let Some(socket) = self.settlement_socket.clone() {
            driver.set_claim_verifier(production_claim_verifier(socket));
        }
        let (host, funnel) = match self.funnel_builder.as_ref() {
            Some(builder) => {
                let funnel = builder(intent, &campaign, host)?;
                (
                    Box::new(UnqualifiedSteamHost) as Box<dyn AttemptHost + Send>,
                    Some(funnel),
                )
            }
            None => (host, None),
        };
        self.driver = Some(DriverState {
            driver,
            host,
            campaign,
            funnel,
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
        // Admission needs the desktop idle by both signals. Once an attempt runs, the game raises its
        // own idle inhibitor, so only genuine user input may interrupt it.
        #[cfg(feature = "wayland-probe")]
        let (idle_at, input_idle_at) = match self.desktop_idle.1 {
            crate::autotune::research::desktop_idle::DesktopIdleObservation::Monitoring {
                inhibitor_idle,
                input_idle,
            } => (
                (inhibitor_idle && input_idle).then_some(self.desktop_idle.0),
                input_idle.then_some(self.desktop_idle.0),
            ),
            _ => (None, None),
        };
        #[cfg(not(feature = "wayland-probe"))]
        let (idle_at, input_idle_at) = (None, None);
        let Some(state) = self.driver.as_mut() else {
            return;
        };
        let active = &mut self.active;
        // The lease must durably record its launching phase before any launch is armed.
        let mut fence = || {
            let active = active
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("campaign lease unavailable"))?;
            anyhow::ensure!(
                active.lease.phase() == ResearchPhase::AwaitSafeWindow,
                "campaign is not ready to launch"
            );
            active.lease.advance(ResearchPhase::Launching, now)
        };
        // The child-study funnel, when this campaign runs in that mode, is ticked instead of the
        // flat driver and reports the same `Step`s, so every lease transition below is shared.
        let ticked = if let Some(funnel) = state.funnel.as_mut() {
            funnel.host_mut().observe_desktop_idle(idle_at);
            funnel.host_mut().observe_user_input_idle(input_idle_at);
            funnel.tick_at(now, &mut fence)
        } else {
            state.host.observe_desktop_idle(idle_at);
            state.host.observe_user_input_idle(input_idle_at);
            state
                .driver
                .tick_with_launch_fence(state.host.as_mut(), now, fence)
        };
        let step = match ticked {
            Ok(step) => step,
            Err(error) => {
                log::error!("research driver failed: {error:#}");
                if error.downcast_ref::<LaunchUnresolved>().is_some() {
                    // A sent request may still start: a no-op abort would prove nothing and
                    // generic abort semantics must not run. Ownership is retained as is.
                    self.launch_unresolved = true;
                } else if let Err(settlement) = state.abort_host() {
                    log::error!("research process settlement is unverified: {settlement:#}");
                }
                // A failed tick may have left a started marker or torn evidence even if the host
                // stopped. Keep the lease and mutation slot until independent recovery proves it.
                self.storage_blocker = Some("UnsafeToResume");
                return;
            }
        };
        if matches!(step, Step::Done) {
            self.finalize_done(now);
            return;
        }
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
            Step::Done => false,
            Step::Expired => {
                self.on_deadline(now);
                true
            }
            Step::ActiveBudgetExhausted => {
                let code = if self.persist_report() {
                    "ActiveBudgetExceeded"
                } else {
                    "ReportPersistenceFailed"
                };
                self.settle(now, ResearchPhase::Blocked, code, None);
                true
            }
            // A durable GPU health stop: preserve what was sealed, leave the rest of the schedule
            // unexecuted, and end the campaign as a typed health block.
            Step::OwnershipUnresolved(code) => {
                // Settlement or restoration is unproven: a terminal transition would release the
                // action slot over a possibly running game, worker transaction or edited settings
                // file. Keep the lease and every mutation suppressed, as for a driver error, and
                // let independent recovery settle it.
                log::error!("research halted with unresolved ownership: {code}");
                self.persist_report();
                if let Some(state) = self.driver.as_mut()
                    && let Err(error) = state.abort_host()
                {
                    log::error!("research process settlement is unverified: {error:#}");
                }
                self.storage_blocker = Some("UnsafeToResume");
                return;
            }
            Step::Stopped(class) => {
                use crate::autotune::research::driver::FailureClass;
                let report_persisted = self.persist_report();
                let terminal = if class == FailureClass::Integrity {
                    ResearchPhase::Aborted
                } else {
                    ResearchPhase::Blocked
                };
                self.settle(now, terminal, class.terminal_code(), Some(class));
                if let Some(terminal) = self.terminal.as_mut() {
                    terminal.report_persisted = Some(report_persisted);
                }
                true
            }
            Step::Halted(code) => {
                let report_persisted = self.persist_report();
                self.settle(now, ResearchPhase::Blocked, code, None);
                if let Some(terminal) = self.terminal.as_mut() {
                    terminal.report_persisted = Some(report_persisted);
                }
                true
            }
            Step::Cancelled => {
                let code = if self.persist_report() {
                    "Cancelled"
                } else {
                    "ReportPersistenceFailed"
                };
                self.settle(now, ResearchPhase::Aborted, code, None);
                true
            }
        };
        if !ok {
            if let Some(state) = self.driver.as_mut()
                && let Err(error) = state.abort_host()
            {
                log::error!("research process settlement is unverified: {error:#}");
            }
            self.storage_blocker = Some("UnsafeToResume");
        }
    }

    /// Request cancellation of a driven attempt; the next drive tick aborts and seals it.
    /// The action gate of a funnel campaign, for the runtime to consult (absent otherwise).
    /// The telemetry observer of a funnel campaign, for the runtime to report into.
    pub(in crate::autotune::runtime) fn telemetry(
        &self,
    ) -> Option<crate::autotune::research::telemetry_observer::SharedTelemetry> {
        self.driver
            .as_ref()
            .and_then(|state| state.funnel.as_ref())
            .and_then(FunnelDriver::telemetry)
    }

    pub(in crate::autotune::runtime) fn action_gate(
        &self,
    ) -> Option<crate::autotune::research::action_gate::SharedActionGate> {
        self.driver
            .as_ref()
            .and_then(|state| state.funnel.as_ref())
            .and_then(FunnelDriver::action_gate)
    }

    pub(super) fn request_driver_cancel(&mut self) -> bool {
        match self.driver.as_mut() {
            Some(state) => {
                match state.funnel.as_mut() {
                    Some(funnel) => funnel.cancel(),
                    None => state.driver.cancel(),
                }
                true
            }
            None => false,
        }
    }

    /// Prove that an attached driver has settled its attempt and any profile it owns.
    /// A lease phase alone is not proof: a funnel can apply a profile before a launch window.
    pub(super) fn settle_owned_driver(
        &mut self,
        now: u64,
    ) -> anyhow::Result<Option<crate::autotune::research::driver::FailureClass>> {
        if self.launch_unresolved || self.storage_blocker == Some("UnsafeToResume") {
            anyhow::bail!("research ownership requires independent recovery");
        }
        let Some(state) = self.driver.as_mut() else {
            return Ok(None);
        };
        if let Some(funnel) = state.funnel.as_mut() {
            funnel
                .shutdown()
                .map_err(|reason| anyhow::anyhow!("funnel settlement unproven: {reason}"))?;
            return Ok(funnel.take_terminal_failure());
        } else {
            state.driver.cancel();
            match state
                .driver
                .tick_with_launch_fence(state.host.as_mut(), now, || {
                    anyhow::bail!("settlement must not launch an attempt")
                })? {
                Step::Sealed(record) => return Ok(record.outcome.terminal_failure()),
                Step::Stopped(class) => return Ok(Some(class)),
                Step::Cancelled | Step::Halted(_) => {}
                _ => anyhow::bail!("research attempt settlement unproven"),
            }
        }
        Ok(None)
    }

    pub(super) fn driver_is_quiescent(&self) -> bool {
        self.driver.as_ref().is_none_or(|state| {
            !state.driver.has_owned_attempt()
                && state.funnel.as_ref().is_none_or(FunnelDriver::is_quiescent)
        })
    }

    /// Settle an in-process attempt before the controller and worker shut down. An unsealed or
    /// previously uncertain attempt remains recoverable only through the existing safety gate.
    pub(in crate::autotune::runtime) fn shutdown_driver(&mut self, now: u64) -> anyhow::Result<()> {
        let Some(state) = self.driver.as_mut() else {
            return Ok(());
        };
        if self.launch_unresolved {
            anyhow::bail!(
                "research launch request is unresolved and requires independent recovery"
            );
        }
        if self.storage_blocker == Some("UnsafeToResume") {
            state.abort_host()?;
            anyhow::bail!("research attempt requires independent recovery");
        }
        let cause = self.settle_owned_driver(now)?;
        let report_persisted = self.persist_report();
        let code = if let Some(class) = cause {
            class.terminal_code()
        } else if report_persisted {
            "ControllerShutdown"
        } else {
            "ReportPersistenceFailed"
        };
        let phase = if cause.is_some_and(|class| {
            class != crate::autotune::research::driver::FailureClass::Integrity
        }) {
            ResearchPhase::Blocked
        } else {
            ResearchPhase::Aborted
        };
        self.settle(now, phase, code, cause);
        if let Some(terminal) = self.terminal.as_mut() {
            terminal.report_persisted = Some(report_persisted);
        }
        anyhow::ensure!(
            self.terminal
                .as_ref()
                .is_some_and(|terminal| terminal.phase == phase),
            "research lease shutdown was not persisted"
        );
        Ok(())
    }

    pub(super) fn settle(
        &mut self,
        now: u64,
        terminal: ResearchPhase,
        code: &'static str,
        cause: Option<crate::autotune::research::driver::FailureClass>,
    ) {
        if let Some(active) = self.active.as_mut() {
            let phase = active.lease.phase();
            if !matches!(
                phase,
                ResearchPhase::Completed | ResearchPhase::Blocked | ResearchPhase::Aborted
            ) {
                let restoring = phase == ResearchPhase::Restoring
                    || active.lease.advance(ResearchPhase::Restoring, now).is_ok();
                if !restoring {
                    self.storage_blocker = Some("UnsafeToResume");
                    return;
                }
                let terminal_result =
                    if cause == Some(crate::autotune::research::driver::FailureClass::Health) {
                        active.lease.block_for_health(now)
                    } else {
                        active.lease.advance(terminal, now)
                    };
                if terminal_result.is_err() {
                    self.storage_blocker = Some("UnsafeToResume");
                    return;
                }
            }
            if active.lease.phase() != terminal {
                self.storage_blocker = Some("UnsafeToResume");
                return;
            }
        }
        self.settle_terminal(terminal, code, cause);
    }

    fn settle_terminal(
        &mut self,
        phase: ResearchPhase,
        code: &'static str,
        cause: Option<crate::autotune::research::driver::FailureClass>,
    ) {
        if let Some(active) = self.active.take() {
            self.terminal = Some(TerminalCampaign {
                phase,
                intent_sha256: active.intent_sha256,
                code,
                report_persisted: None,
                finalization_verified: None,
                primary_cause: cause.map(|class| class.terminal_code()),
            });
        }
        self.driver = None;
    }
}

/// The host factory the resident session installs. The composed production host is selected only
/// when `research_live_launch` is explicitly enabled and every research source is configured;
/// anything else keeps the unqualified host, which never reports a safe window and never launches.
pub(in crate::autotune::runtime) fn select_host_factory(
    config: &crate::daemon::config::autotune::DaemonAutotuneConfig,
) -> HostFactory {
    use crate::autotune::research::session_host::{SessionHostSources, build_session_host};
    let sources = match (
        config.research_live_launch,
        config.research_study_dir.clone(),
        config.research_steamapps_dir.clone(),
        config.research_proton_manifest.clone(),
    ) {
        (true, Some(study_dir), Some(steamapps), Some(proton_manifest)) => {
            Some(SessionHostSources {
                study_dir,
                steamapps,
                proton_manifest,
                worker_socket: config.privileged_worker_socket.clone().or_else(|| {
                    crate::daemon::privilege::default_privileged_worker_socket_path().ok()
                }),
            })
        }
        _ => None,
    };
    match sources {
        Some(sources) => Box::new(move |intent| build_session_host(&sources, intent)),
        None => Box::new(|_| Ok(Box::new(UnqualifiedSteamHost))),
    }
}

/// The recovery witnesses the resident session supplies, under the same opt-in and with the same
/// fully configured sources as the live host. Without them, launch history stays unsafe.
pub(in crate::autotune::runtime) fn select_recovery_factory(
    config: &crate::daemon::config::autotune::DaemonAutotuneConfig,
) -> Option<crate::autotune::runtime::research_recovery::RecoveryFactory> {
    use crate::autotune::research::session_host::{SessionHostSources, build_recovery_witnesses};
    match (
        config.research_live_launch,
        config.research_study_dir.clone(),
        config.research_steamapps_dir.clone(),
        config.research_proton_manifest.clone(),
    ) {
        (true, Some(study_dir), Some(steamapps), Some(proton_manifest)) => {
            let sources = SessionHostSources {
                study_dir,
                steamapps,
                proton_manifest,
                worker_socket: config.privileged_worker_socket.clone().or_else(|| {
                    crate::daemon::privilege::default_privileged_worker_socket_path().ok()
                }),
            };
            Some(std::sync::Arc::new(move |intent| {
                build_recovery_witnesses(&sources, intent)
            }))
        }
        _ => None,
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
            class: crate::autotune::research::driver::FailureClass::Environment,
            code: "BlockedSteamAutomation".into(),
        }
    }
    fn abort(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
}

impl ResearchCoordinator {
    /// Write `report.json` for a finished schedule. Called while the driver still owns the journal.
    pub(super) fn persist_report(&mut self) -> bool {
        let Some(state) = self.driver.as_ref() else {
            return false;
        };
        let result = state.driver.finalize().and_then(|report| {
            crate::autotune::research::report_store::write_report(&state.campaign, &report)
        });
        if let Err(error) = &result {
            log::error!("research report could not be persisted: {error:#}");
        }
        // The child studies this build cannot execute are decided and recorded, not left implicit.
        // A ledger that cannot be written means the campaign's accounting is incomplete, so the
        // report is not treated as persisted.
        if result.is_ok() && state.funnel.is_none() {
            let intent = state.driver.intent();
            if let Err(error) = crate::autotune::research::child_ledger::settle_unexecuted(
                &state.campaign,
                intent.uid,
                intent,
            ) {
                log::error!("child-study dispositions could not be recorded: {error:#}");
                return false;
            }
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
}
