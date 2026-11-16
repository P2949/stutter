//! Resident attempt driving. The coordinator owns one `Driver` per admitted campaign and advances
//! the durable lease phases only from what the driver observed. Without a host factory (tests,
//! unqualified builds) no attempt is ever scheduled.
use std::{fs::File, path::PathBuf};

use super::{ActiveCampaign, ResearchCoordinator, TerminalCampaign};
use crate::{
    autotune::research::{
        ResearchIntentV1, ResearchJournal, ResearchPhase,
        driver::{AttemptHost, Driver, LaunchUnresolved, Step, build_schedule},
        funnel_driver::FunnelDriver,
        settlement::ProductionSettlementVerifier,
    },
    daemon::privilege::UnixSocketPrivilegedActionService,
};

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
            driver.set_claim_verifier(Box::new(ProductionSettlementVerifier::new(
                "/proc".into(),
                Some(Box::new(move || {
                    UnixSocketPrivilegedActionService::new(socket.clone()).outstanding_tokens()
                })),
            )));
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
        // A complete schedule is not a verified campaign. Persist the report before any
        // terminal lease transition, and retain the explicit provenance blocker until the
        // independent verification gate is implemented.
        let done_code = matches!(step, Step::Done).then(|| {
            if self.persist_report() {
                "IncompleteProvenance"
            } else {
                "ReportPersistenceFailed"
            }
        });
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
            Step::Done => advance(active, &[ResearchPhase::Restoring, ResearchPhase::Blocked]),
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
                self.settle(now, ResearchPhase::Blocked, code);
                true
            }
            // A durable GPU health stop: preserve what was sealed, leave the rest of the schedule
            // unexecuted, and end the campaign as a typed health block.
            Step::Halted(code) => {
                let code = if self.persist_report() {
                    code
                } else {
                    "ReportPersistenceFailed"
                };
                self.settle(now, ResearchPhase::Blocked, code);
                true
            }
            Step::Cancelled => {
                let code = if self.persist_report() {
                    "Cancelled"
                } else {
                    "ReportPersistenceFailed"
                };
                self.settle(now, ResearchPhase::Aborted, code);
                true
            }
        };
        if let Some(code) = done_code.filter(|_| ok) {
            self.settle_terminal(ResearchPhase::Blocked, code);
        } else if !ok {
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
        if let Some(funnel) = state.funnel.as_mut() {
            // Settle the running attempt, then restore and prove the profile; unproven is a refusal.
            funnel
                .shutdown()
                .map_err(|reason| anyhow::anyhow!("research funnel shutdown unproven: {reason}"))?;
        } else {
            state.driver.cancel();
            match state
                .driver
                .tick_with_launch_fence(state.host.as_mut(), now, || {
                    anyhow::bail!("shutdown must not start an attempt")
                })? {
                Step::Sealed(_) | Step::Cancelled | Step::Halted(_) => {}
                _ => anyhow::bail!("research shutdown did not settle the attempt"),
            }
        }
        let code = if self.persist_report() {
            "ControllerShutdown"
        } else {
            "ReportPersistenceFailed"
        };
        self.settle(now, ResearchPhase::Aborted, code);
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

    /// Replay the sealed native result files of the campaign in `dir` under each arm's own prefix.
    fn raw_replay(
        &self,
        dir: &File,
        uid: u32,
    ) -> (
        serde_json::Value,
        Option<crate::autotune::research::raw_verify::RawVerification>,
    ) {
        use std::os::fd::AsFd;

        use crate::autotune::research::{
            ResearchIntentV1, ResearchJournal, driver::build_schedule, raw_verify,
            report_store::read_private, session_host,
        };
        let replay = || -> anyhow::Result<raw_verify::RawVerification> {
            let study = self
                .study_dir
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("no study directory"))?;
            let intent: ResearchIntentV1 =
                serde_json::from_slice(&read_private(dir, "intent.json", uid)?)?;
            let journal = ResearchJournal::open(
                crate::syscall::openat_dir_nofollow(dir.as_fd(), "journal")?,
                uid,
                build_schedule(&intent)?,
            )?;
            let records = journal.verify()?;
            let evidence = File::from(crate::syscall::openat_dir_nofollow(
                dir.as_fd(),
                "evidence",
            )?);
            raw_verify::verify_raw_evidence(&evidence, uid, &records, &|arm| {
                session_host::arm_prefix_paths(study, arm).1
            })
        };
        match replay() {
            Ok(outcome) => (
                serde_json::json!({
                    "status": if outcome.passed() { "replayed" } else { "failed" },
                    "detail": outcome,
                }),
                Some(outcome),
            ),
            Err(error) => (
                serde_json::json!({ "status": "error", "reason": format!("{error:#}") }),
                None,
            ),
        }
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
                // The raw native files the sealed records name are reopened and re-hashed too. Each
                // verification status stands on its own evidence (see `verdict`); the response
                // stays `IncompleteProvenance` until the final gate is genuinely qualified.
                let (raw, raw_outcome) = self.raw_replay(&dir, uid);
                let intent =
                    crate::autotune::research::report_store::read_private(&dir, "intent.json", uid)
                        .ok()
                        .and_then(|bytes| {
                            serde_json::from_slice::<crate::autotune::research::ResearchIntentV1>(
                                &bytes,
                            )
                            .ok()
                        });
                let studies = intent.as_ref().and_then(|intent| {
                    crate::autotune::research::child_ledger::read_dispositions(&dir, uid, intent)
                        .ok()
                });
                // Each child study's own report and raw files, verified like the campaign's.
                let children =
                    intent
                        .as_ref()
                        .zip(self.study_dir.as_deref())
                        .and_then(|(intent, study)| {
                            crate::autotune::research::child_verify::verify_children(
                                &dir,
                                uid,
                                intent,
                                &|arm| {
                                    crate::autotune::research::session_host::arm_prefix_paths(
                                        study, arm,
                                    )
                                    .1
                                },
                            )
                            .ok()
                        });
                let evidence: Vec<_> = {
                    use std::os::fd::AsFd;
                    crate::syscall::openat_dir_nofollow(dir.as_fd(), "evidence")
                        .ok()
                        .map(File::from)
                        .and_then(|evidence_dir| {
                            crate::autotune::research::report_store::read_evidence(
                                &evidence_dir,
                                uid,
                            )
                            .ok()
                        })
                        .map(|items| items.into_values().collect())
                        .unwrap_or_default()
                };
                let health_blocked = self.study_dir.as_deref().is_none_or(|study| {
                    crate::autotune::research::gpu_health::state_blocked(study)
                });
                let verdict = crate::autotune::research::verdict::judge(
                    &crate::autotune::research::verdict::VerdictInputs {
                        reconstruction_passed: true,
                        raw: raw_outcome.as_ref(),
                        evidence: &evidence,
                        child_studies: studies.as_deref().unwrap_or(&[]),
                        child_reports_verified: children
                            .as_ref()
                            .is_some_and(|children| children.all_verified()),
                        // No physical restoration verifier exists yet; never inferred.
                        restoration_verified: false,
                        health_blocked,
                    },
                );
                let body = serde_json::json!({
                    "status": if verdict.final_gate_qualified { "ok" } else { "blocked" },
                    "code": if verdict.final_gate_qualified { "Qualified" } else { "IncompleteProvenance" },
                    "reconstruction": "passed",
                    "verdict": verdict,
                    "raw_native_files": raw,
                    "child_studies": studies,
                    "child_reports": children,
                    "intent_sha256": report.intent_sha256,
                });
                // The named final artifacts, once, beside the report; a failure is not a verdict.
                crate::autotune::research::final_bundle::write_logged(&dir, &report, &body);
                body.to_string() + "\n"
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
