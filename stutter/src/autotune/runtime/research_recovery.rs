//! Read-only campaign recovery work, executed away from the monitor loop.
use std::{
    fs::File,
    os::{
        fd::{AsFd, AsRawFd},
        unix::fs::MetadataExt,
    },
    path::{Path, PathBuf},
};

use crate::autotune::research::{CampaignFacts, CampaignLease, ResearchPhase};

pub(super) struct RecoveryJob {
    pub(super) dir: PathBuf,
    pub(super) facts: Option<CampaignFacts>,
    pub(super) now: u64,
    pub(super) study_dir: Option<PathBuf>,
}

pub(super) struct RecoveredCampaign {
    pub(super) facts: Option<CampaignFacts>,
    pub(super) lease: CampaignLease,
    pub(super) study_dir: Option<PathBuf>,
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
        verify_no_attempts(&lease, self.study_dir.as_deref())?;
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
            study_dir: self.study_dir,
        })
    }
}

/// A lease can still say AwaitSafeWindow after the driver has journalled and launched an attempt.
/// The separate study journal must therefore independently prove that no attempt was started.
fn verify_no_attempts(lease: &CampaignLease, study_path: Option<&Path>) -> anyhow::Result<()> {
    let Some(expected) = lease.study_root() else {
        anyhow::ensure!(study_path.is_none(), "UnsafeToResume: study root changed");
        return Ok(());
    };
    let path =
        study_path.ok_or_else(|| anyhow::anyhow!("UnsafeToResume: study root unavailable"))?;
    let uid = crate::syscall::geteuid();
    let root = File::from(crate::syscall::open_dir_no_symlinks(path)?);
    let metadata = root.metadata()?;
    anyhow::ensure!(
        metadata.is_dir()
            && metadata.uid() == uid
            && metadata.mode() & 0o077 == 0
            && metadata.dev() == expected.device
            && metadata.ino() == expected.inode,
        "UnsafeToResume: study root identity changed"
    );
    let campaign = File::from(crate::syscall::openat_dir_nofollow(
        root.as_fd(),
        lease
            .intent_sha256()
            .get(..16)
            .ok_or_else(|| anyhow::anyhow!("invalid digest"))?,
    )?);
    let metadata = campaign.metadata()?;
    anyhow::ensure!(
        metadata.uid() == uid && metadata.mode() & 0o077 == 0,
        "untrusted study campaign"
    );
    let bytes =
        crate::autotune::research::report_store::read_private(&campaign, "intent.json", uid)?;
    let intent: crate::autotune::research::ResearchIntentV1 = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(
        intent.uid == uid
            && intent.study_root == Some(expected)
            && intent.digest()? == lease.intent_sha256(),
        "UnsafeToResume: study intent mismatch"
    );
    for name in ["journal", "dispatch"] {
        let dir = File::from(crate::syscall::openat_dir_nofollow(campaign.as_fd(), name)?);
        let metadata = dir.metadata()?;
        anyhow::ensure!(
            metadata.uid() == uid && metadata.mode() & 0o077 == 0,
            "untrusted study directory"
        );
        anyhow::ensure!(
            std::fs::read_dir(format!("/proc/self/fd/{}", dir.as_raw_fd()))?
                .next()
                .is_none(),
            "UnsafeToResume: attempt records require settlement verification"
        );
    }
    Ok(())
}
