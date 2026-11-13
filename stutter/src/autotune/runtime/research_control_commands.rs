//! Pause and resume at safe campaign boundaries.
use super::{ResearchCoordinator, response};
use crate::autotune::research::ResearchPhase;

impl ResearchCoordinator {
    pub(super) fn pause(&mut self, now: u64) -> String {
        if self.storage_blocker == Some("UnsafeToResume") {
            return response("blocked", "UnsafeToResume");
        }
        let Some(active) = self.active.as_mut() else {
            return response("denied", "NoCampaign");
        };
        if active.lease.phase() == ResearchPhase::Paused {
            return self.status();
        }
        if active.lease.phase() != ResearchPhase::AwaitSafeWindow {
            return response("blocked", "CannotPauseActiveStage");
        }
        if active.lease.advance(ResearchPhase::Paused, now).is_err() {
            return response("blocked", "PausePersistenceFailed");
        }
        self.status()
    }

    pub(super) fn resume(&mut self, now: u64) -> String {
        if self.storage_blocker == Some("UnsafeToResume") {
            return response("blocked", "UnsafeToResume");
        }
        let Some(active) = self.active.as_mut() else {
            return response("denied", "NoCampaign");
        };
        if active.lease.phase() == ResearchPhase::AwaitSafeWindow {
            return self.status();
        }
        if active.lease.phase() != ResearchPhase::Paused {
            return response("blocked", "CannotResumeActiveStage");
        }
        if active
            .lease
            .advance(ResearchPhase::AwaitSafeWindow, now)
            .is_err()
        {
            return response("blocked", "ResumePersistenceFailed");
        }
        self.status()
    }
}
