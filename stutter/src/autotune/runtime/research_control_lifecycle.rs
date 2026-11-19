//! Cancellation and deadline settlement for the resident research coordinator.

use super::*;

impl ResearchCoordinator {
    pub(super) fn cancel(&mut self, now: u64) -> String {
        if self.storage_blocker == Some("UnsafeToResume") {
            return response("blocked", "UnsafeToResume");
        }
        let Some(active) = self.active.as_mut() else {
            return response("denied", "NoCampaign");
        };
        if matches!(
            active.lease.phase(),
            ResearchPhase::Completed | ResearchPhase::Blocked | ResearchPhase::Aborted
        ) {
            return self.status();
        }
        if matches!(
            active.lease.phase(),
            ResearchPhase::Launching
                | ResearchPhase::Attesting
                | ResearchPhase::Warmup
                | ResearchPhase::Measuring
        ) && self.request_driver_cancel()
        {
            return response("accepted", "CancellationRequested");
        }
        let Some(active) = self.active.as_mut() else {
            return response("denied", "NoCampaign");
        };
        if !matches!(
            active.lease.phase(),
            ResearchPhase::Requested
                | ResearchPhase::Preflight
                | ResearchPhase::Prepared
                | ResearchPhase::AwaitSafeWindow
                | ResearchPhase::Paused
        ) {
            return response("blocked", "CancellationNeedsExecutorSettlement");
        }
        let cause = match self.settle_owned_driver(now) {
            Ok(cause) => cause,
            Err(error) => {
                log::error!("research cancellation could not settle owned resources: {error:#}");
                self.storage_blocker = Some("UnsafeToResume");
                return response("blocked", "UnsafeToResume");
            }
        };
        let report_persisted = self.driver.is_some().then(|| self.persist_report());
        let report_failed = report_persisted == Some(false);
        let phase = if cause.is_some_and(|class| {
            class != crate::autotune::research::driver::FailureClass::Integrity
        }) {
            ResearchPhase::Blocked
        } else {
            ResearchPhase::Aborted
        };
        let Some(active) = self.active.as_mut() else {
            return response("denied", "NoCampaign");
        };
        if active.lease.advance(ResearchPhase::Restoring, now).is_err()
            || if cause == Some(crate::autotune::research::driver::FailureClass::Health) {
                active.lease.block_for_health(now).is_err()
            } else {
                active.lease.advance(phase, now).is_err()
            }
        {
            return response("blocked", "CancellationPersistenceFailed");
        }
        self.terminal = Some(TerminalCampaign {
            phase,
            intent_sha256: active.intent_sha256.clone(),
            report_persisted,
            finalization_verified: None,
            primary_cause: cause.map(|class| class.terminal_code()),
            code: if let Some(class) = cause {
                class.terminal_code()
            } else if report_failed {
                "ReportPersistenceFailed"
            } else {
                "Cancelled"
            },
        });
        self.active = None;
        self.driver = None;
        self.status()
    }

    /// Expire only a quiescent boundary. An active executor must settle its own resources first.
    pub(in crate::autotune::runtime) fn deadline_due(&self, now: u64) -> bool {
        if self.storage_blocker == Some("UnsafeToResume") {
            return false;
        }
        self.active.as_ref().is_some_and(|active| {
            now >= active.deadline_unix_seconds
                && matches!(
                    active.lease.phase(),
                    ResearchPhase::Requested
                        | ResearchPhase::Preflight
                        | ResearchPhase::Prepared
                        | ResearchPhase::AwaitSafeWindow
                        | ResearchPhase::Paused
                )
        })
    }

    pub(in crate::autotune::runtime) fn on_deadline(&mut self, now: u64) {
        if !self.deadline_due(now) {
            return;
        }
        let cause = match self.settle_owned_driver(now) {
            Ok(cause) => cause,
            Err(error) => {
                log::error!("research deadline could not settle owned resources: {error:#}");
                self.storage_blocker = Some("UnsafeToResume");
                return;
            }
        };
        let report_persisted = self.driver.is_some().then(|| self.persist_report());
        let report_failed = report_persisted == Some(false);
        let phase = if cause == Some(crate::autotune::research::driver::FailureClass::Integrity) {
            ResearchPhase::Aborted
        } else {
            ResearchPhase::Blocked
        };
        let Some(active) = self.active.as_mut() else {
            return;
        };
        if active.lease.advance(ResearchPhase::Restoring, now).is_err()
            || if cause == Some(crate::autotune::research::driver::FailureClass::Health) {
                active.lease.block_for_health(now).is_err()
            } else if cause.is_some() {
                active.lease.advance(phase, now).is_err()
            } else {
                active.lease.block_for_deadline(now).is_err()
            }
        {
            log::error!("research deadline settlement could not be persisted");
            return;
        }
        self.terminal = Some(TerminalCampaign {
            phase,
            intent_sha256: active.intent_sha256.clone(),
            report_persisted,
            finalization_verified: None,
            primary_cause: cause.map(|class| class.terminal_code()),
            code: if let Some(class) = cause {
                class.terminal_code()
            } else {
                match (report_failed, active.lease.block_reason()) {
                    (true, _) => "ReportPersistenceFailed",
                    (false, Some(CampaignBlockReason::DeadlineExpired)) => "DeadlineExpired",
                    (false, Some(CampaignBlockReason::GpuHealthIncident)) => "BlockedGpuStability",
                    (false, None) => "Blocked",
                }
            },
        });
        self.active = None;
        self.driver = None;
    }
}
