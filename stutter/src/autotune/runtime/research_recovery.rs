//! Read-only campaign recovery work, executed away from the monitor loop.
use std::path::PathBuf;

use crate::autotune::research::{CampaignFacts, CampaignLease, ResearchPhase};

pub(super) struct RecoveryJob {
    pub(super) dir: PathBuf,
    pub(super) facts: Option<CampaignFacts>,
    pub(super) now: u64,
}

pub(super) struct RecoveredCampaign {
    pub(super) facts: Option<CampaignFacts>,
    pub(super) lease: CampaignLease,
}

impl RecoveryJob {
    /// Only prelaunch phases are recoverable until child-study and worker settlement are wired.
    pub(super) fn run(self) -> anyhow::Result<RecoveredCampaign> {
        let dir = crate::syscall::open_dir_no_symlinks(&self.dir)?;
        let lease = match &self.facts {
            Some(facts) => CampaignLease::recover(dir, facts, self.now)?,
            None => {
                CampaignLease::recover_terminal_prelaunch(dir, crate::syscall::geteuid(), self.now)?
            }
        };
        anyhow::ensure!(
            !lease.has_started_launch(),
            "UnsafeToResume: launch history needs attempt and worker verification"
        );
        anyhow::ensure!(
            matches!(
                lease.phase(),
                ResearchPhase::AwaitSafeWindow
                    | ResearchPhase::Paused
                    | ResearchPhase::Blocked
                    | ResearchPhase::Aborted
            ),
            "UnsafeToResume: unsupported recovered phase"
        );
        Ok(RecoveredCampaign {
            facts: self.facts,
            lease,
        })
    }
}
