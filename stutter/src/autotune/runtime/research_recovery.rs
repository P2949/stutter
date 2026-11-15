//! Campaign recovery work, executed away from the monitor loop. A campaign that never launched is
//! recovered read-only. A campaign with launch history is recovered only when a witness factory
//! is supplied: the interrupted attempt is reconstructed from durable records and sealed failed
//! if, and only if, its launch, process tree, worker and restoration are independently settled.
use std::{
    fs::File,
    os::{
        fd::{AsFd, AsRawFd},
        unix::fs::MetadataExt,
    },
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::autotune::research::{
    CampaignFacts, CampaignLease, ResearchIntentV1, ResearchJournal, ResearchPhase,
    driver::{Driver, build_schedule},
    recovery_gather::RecoveryWitnesses,
};

/// Builds the independent witnesses for one frozen intent, or fails when a source is unavailable.
pub(in crate::autotune::runtime) type RecoveryFactory =
    Arc<dyn Fn(&ResearchIntentV1) -> anyhow::Result<RecoveryWitnesses> + Send + Sync>;

pub(super) struct RecoveryJob {
    pub(super) dir: PathBuf,
    pub(super) facts: Option<CampaignFacts>,
    pub(super) now: u64,
    pub(super) study_dir: Option<PathBuf>,
    pub(super) witnesses: Option<RecoveryFactory>,
}

pub(super) struct RecoveredCampaign {
    pub(super) facts: Option<CampaignFacts>,
    pub(super) lease: CampaignLease,
    pub(super) study_dir: Option<PathBuf>,
}

impl RecoveryJob {
    pub(super) fn run(self) -> anyhow::Result<RecoveredCampaign> {
        let dir = crate::syscall::open_dir_no_symlinks(&self.dir)?;
        let mut lease = match &self.facts {
            // An in-flight phase is only recoverable when witnesses can retire the attempt.
            Some(facts) if self.witnesses.is_some() => {
                CampaignLease::recover_interrupted_launch(dir, facts, self.now)?
            }
            Some(facts) => CampaignLease::recover(dir, facts, self.now)?,
            None => {
                CampaignLease::recover_terminal_prelaunch(dir, crate::syscall::geteuid(), self.now)?
            }
        };
        if lease.has_started_launch() {
            // Launch history needs the attempt, the process tree and the worker verified; without a
            // witness factory it stays unsafe, exactly as before.
            let factory = self.witnesses.as_ref().ok_or_else(|| {
                anyhow::anyhow!(
                    "UnsafeToResume: launch history needs attempt and worker verification"
                )
            })?;
            recover_launched(&mut lease, self.study_dir.as_deref(), factory, self.now)?;
            return Ok(RecoveredCampaign {
                facts: self.facts,
                lease,
                study_dir: self.study_dir,
            });
        }
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

/// Open the study's campaign directory and its frozen intent, re-verifying ownership, identity
/// and that the intent still matches the lease.
fn open_campaign(
    lease: &CampaignLease,
    study_path: Option<&Path>,
) -> anyhow::Result<Option<(File, ResearchIntentV1)>> {
    let Some(expected) = lease.study_root() else {
        anyhow::ensure!(study_path.is_none(), "UnsafeToResume: study root changed");
        return Ok(None);
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
    let intent: ResearchIntentV1 = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(
        intent.uid == uid
            && intent.study_root == Some(expected)
            && intent.digest()? == lease.intent_sha256(),
        "UnsafeToResume: study intent mismatch"
    );
    Ok(Some((campaign, intent)))
}

/// A lease can still say AwaitSafeWindow after the driver has journalled and launched an attempt.
/// The separate study journal must therefore independently prove that no attempt was started.
fn verify_no_attempts(lease: &CampaignLease, study_path: Option<&Path>) -> anyhow::Result<()> {
    let Some((campaign, _)) = open_campaign(lease, study_path)? else {
        return Ok(());
    };
    for name in ["journal", "dispatch"] {
        let dir = File::from(crate::syscall::openat_dir_nofollow(campaign.as_fd(), name)?);
        let metadata = dir.metadata()?;
        anyhow::ensure!(
            metadata.uid() == crate::syscall::geteuid() && metadata.mode() & 0o077 == 0,
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

/// Retire the one interrupted attempt of a launched campaign and block the campaign. The attempt
/// is never replayed and the remaining attempts are not resumed: the consumed active time is not
/// recoverable. Any witness that cannot be produced or does not agree leaves the campaign unsafe.
fn recover_launched(
    lease: &mut CampaignLease,
    study_path: Option<&Path>,
    factory: &RecoveryFactory,
    now: u64,
) -> anyhow::Result<()> {
    let (campaign, intent) = open_campaign(lease, study_path)?
        .ok_or_else(|| anyhow::anyhow!("UnsafeToResume: launch history without a study root"))?;
    let uid = crate::syscall::geteuid();
    let open = |name: &str| -> anyhow::Result<File> {
        let dir = File::from(crate::syscall::openat_dir_nofollow(campaign.as_fd(), name)?);
        let metadata = dir.metadata()?;
        anyhow::ensure!(
            metadata.uid() == uid && metadata.mode() & 0o077 == 0,
            "untrusted study directory"
        );
        Ok(dir)
    };
    let (journal_dir, dispatch, evidence) =
        (open("journal")?, open("dispatch")?, open("evidence")?);
    let schedule = build_schedule(&intent)?;
    let len = schedule.len();
    let (journal, adopted) = ResearchJournal::open_for_recovery(journal_dir.into(), uid, schedule)?;
    let witnesses = factory(&intent)?;
    let same_boot = crate::telemetry::identity::read_boot_id()? == intent.boot_id;
    let mut driver = Driver::new(journal, intent, dispatch, evidence, len);
    if let Some(slot) = adopted {
        driver.recover_interrupted(slot, same_boot, witnesses, now)?;
    }
    // Every attempt that started is now sealed. Block the campaign through the normal restoring
    // transition so it ends as a recovered, blocked campaign with its report intact.
    if !matches!(
        lease.phase(),
        ResearchPhase::Completed | ResearchPhase::Blocked | ResearchPhase::Aborted
    ) {
        if lease.phase() != ResearchPhase::Restoring {
            lease.advance(ResearchPhase::Restoring, now)?;
        }
        lease.advance(ResearchPhase::Blocked, now)?;
    }
    Ok(())
}
