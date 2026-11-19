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
    pub(super) terminal_code: Option<&'static str>,
}

fn has_prepared_finalization(
    campaign: &File,
    uid: u32,
    intent_sha256: &str,
) -> anyhow::Result<bool> {
    match crate::autotune::research::report_store::read_private(
        campaign,
        "finalization-prepared.json",
        uid,
    ) {
        Ok(_) => {
            super::research_control::finalize::prepared_record(campaign, uid, intent_sha256)?;
            Ok(true)
        }
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound) =>
        {
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

impl RecoveryJob {
    pub(super) fn run(self) -> anyhow::Result<RecoveredCampaign> {
        let dir = crate::syscall::open_dir_no_symlinks(&self.dir)?;
        let mut historical = false;
        let mut lease = match &self.facts {
            // An in-flight phase is only recoverable when witnesses can retire the attempt.
            Some(facts) if self.witnesses.is_some() => {
                match CampaignLease::recover_interrupted_launch(dir.try_clone()?, facts, self.now) {
                    Ok(lease) => lease,
                    Err(_) => {
                        historical = true;
                        CampaignLease::recover_completed_historical(
                            dir,
                            crate::syscall::geteuid(),
                            self.now,
                        )?
                    }
                }
            }
            Some(facts) => match CampaignLease::recover(dir.try_clone()?, facts, self.now) {
                Ok(lease) => lease,
                Err(_) => {
                    match CampaignLease::recover_interrupted_launch(
                        dir.try_clone()?,
                        facts,
                        self.now,
                    ) {
                        Ok(lease)
                            if !lease.has_started_launch()
                                && lease.phase() == ResearchPhase::Restoring =>
                        {
                            lease
                        }
                        _ => {
                            historical = true;
                            CampaignLease::recover_completed_historical(
                                dir,
                                crate::syscall::geteuid(),
                                self.now,
                            )?
                        }
                    }
                }
            },
            None => match CampaignLease::recover_terminal_prelaunch(
                dir.try_clone()?,
                crate::syscall::geteuid(),
                self.now,
            ) {
                Ok(lease) => lease,
                Err(_) => {
                    historical = true;
                    CampaignLease::recover_completed_historical(
                        dir,
                        crate::syscall::geteuid(),
                        self.now,
                    )?
                }
            },
        };
        if lease.phase() == ResearchPhase::Completed {
            // Do not re-admit old facts or run live cleanup for a historical terminal. The
            // coordinator reserves the action slot until its released artifacts are replayed.
            open_campaign(&lease, self.study_dir.as_deref())?;
            return Ok(RecoveredCampaign {
                facts: if historical { None } else { self.facts },
                lease,
                study_dir: self.study_dir,
                terminal_code: None,
            });
        }
        if lease.has_started_launch() {
            // Launch history needs the attempt, the process tree and the worker verified; without a
            // witness factory it stays unsafe, exactly as before.
            let factory = self.witnesses.as_ref().ok_or_else(|| {
                anyhow::anyhow!(
                    "UnsafeToResume: launch history needs attempt and worker verification"
                )
            })?;
            let terminal_code =
                recover_launched(&mut lease, self.study_dir.as_deref(), factory, self.now)?;
            return Ok(RecoveredCampaign {
                facts: self.facts,
                lease,
                study_dir: self.study_dir,
                terminal_code,
            });
        }
        verify_no_attempts(&lease, self.study_dir.as_deref())?;
        anyhow::ensure!(
            matches!(
                lease.phase(),
                ResearchPhase::AwaitSafeWindow
                    | ResearchPhase::Paused
                    | ResearchPhase::Completed
                    | ResearchPhase::Blocked
                    | ResearchPhase::Aborted
                    | ResearchPhase::Restoring
            ),
            "UnsafeToResume: unsupported recovered phase"
        );
        Ok(RecoveredCampaign {
            facts: self.facts,
            lease,
            study_dir: self.study_dir,
            terminal_code: None,
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
    let Some((campaign, intent)) = open_campaign(lease, study_path)? else {
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
    // A child study's attempts live under `studies/<study>/`, never in the flat directories above.
    // A child that only began (its directories exist) and never started an attempt is fine; one whose
    // journal or dispatch holds anything needs settlement verification.
    let studies = match crate::syscall::openat_dir_nofollow(campaign.as_fd(), "studies") {
        Ok(dir) => Some(File::from(dir)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let mut recovered = Vec::new();
    if let Some(studies) = studies {
        let plan = crate::autotune::research::child_study::plan(&intent)?;
        for entry in std::fs::read_dir(format!("/proc/self/fd/{}", studies.as_raw_fd()))? {
            let name = entry?
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("invalid child study name"))?;
            let study = File::from(crate::syscall::openat_dir_nofollow(studies.as_fd(), &name)?);
            let metadata = study.metadata()?;
            anyhow::ensure!(
                metadata.uid() == crate::syscall::geteuid() && metadata.mode() & 0o077 == 0,
                "untrusted child study directory"
            );
            for part in ["journal", "dispatch", "evidence"] {
                let dir = File::from(crate::syscall::openat_dir_nofollow(study.as_fd(), part)?);
                anyhow::ensure!(
                    std::fs::read_dir(format!("/proc/self/fd/{}", dir.as_raw_fd()))?
                        .next()
                        .is_none(),
                    "UnsafeToResume: child study records require settlement verification"
                );
            }
            let uid = crate::syscall::geteuid();
            let child: ResearchIntentV1 = serde_json::from_slice(
                &crate::autotune::research::report_store::read_private(&study, "intent.json", uid)?,
            )?;
            anyhow::ensure!(
                crate::autotune::research::child_verify::derived_from(&child, &intent, &name),
                "UnsafeToResume: a child study intent differs"
            );
            let planned = plan
                .iter()
                .find(|item| item.study_sha256.get(..16) == Some(name.as_str()))
                .ok_or_else(|| anyhow::anyhow!("UnsafeToResume: child study is not planned"))?
                .clone();
            let journal = ResearchJournal::open(
                crate::syscall::openat_dir_nofollow(study.as_fd(), "journal")?,
                uid,
                build_schedule(&child)?,
            )?;
            let evidence = File::from(crate::syscall::openat_dir_nofollow(
                study.as_fd(),
                "evidence",
            )?);
            let report = crate::autotune::research::report_store::build_from_dirs(
                &child, &journal, &evidence,
            )?;
            recovered.push(RecoveredChild {
                dir: study,
                study: planned,
                report,
            });
        }
        recovered.sort_by_key(|item| {
            plan.iter()
                .position(|study| study.study_sha256 == item.study.study_sha256)
                .unwrap_or(usize::MAX)
        });
    }
    // Even without a launch, a funnel may have applied a profile while awaiting a safe window.
    restore_profiles(study_path, &intent)?;
    let allow_new = !matches!(
        lease.phase(),
        ResearchPhase::Completed | ResearchPhase::Blocked | ResearchPhase::Aborted
    );
    record_recovered_children(
        &campaign,
        crate::syscall::geteuid(),
        &intent,
        recovered,
        allow_new,
    )?;
    if lease.phase() == ResearchPhase::Restoring {
        anyhow::ensure!(
            has_prepared_finalization(&campaign, crate::syscall::geteuid(), lease.intent_sha256())?,
            "UnsafeToResume: restoring lease has no prepared finalization"
        );
    }
    Ok(())
}

struct RecoveredChild {
    dir: File,
    study: crate::autotune::research::child_study::ChildStudy,
    report: crate::autotune::research::report::ResearchReportV1,
}

/// Settle every child study of a funnel campaign: each child's own journal is recovered with the
/// child's own frozen intent, and an unsealed attempt is retired only when every witness agrees.
fn recover_children(
    campaign: &File,
    uid: u32,
    parent: &ResearchIntentV1,
    factory: &RecoveryFactory,
    same_boot: bool,
    now: u64,
    parent_has_attempts: bool,
) -> anyhow::Result<(
    Option<crate::autotune::research::driver::FailureClass>,
    bool,
    Vec<RecoveredChild>,
)> {
    let studies = match crate::syscall::openat_dir_nofollow(campaign.as_fd(), "studies") {
        Ok(dir) => File::from(dir),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((None, false, Vec::new()));
        }
        Err(error) => return Err(error.into()),
    };
    let mut names = Vec::new();
    for entry in std::fs::read_dir(format!("/proc/self/fd/{}", studies.as_raw_fd()))? {
        names.push(entry?.file_name().to_string_lossy().into_owned());
    }
    let plan = crate::autotune::research::child_study::plan(parent)?;
    names.sort_by_key(|name| {
        plan.iter()
            .position(|study| study.study_sha256.get(..16) == Some(name.as_str()))
            .unwrap_or(usize::MAX)
    });
    let mut terminal = None;
    let mut any_attempts = false;
    let mut recovered = Vec::new();
    for name in names {
        let dir = File::from(crate::syscall::openat_dir_nofollow(studies.as_fd(), &name)?);
        let metadata = dir.metadata()?;
        anyhow::ensure!(
            metadata.uid() == uid && metadata.mode() & 0o077 == 0,
            "untrusted child study directory"
        );
        let study = plan
            .iter()
            .find(|study| study.study_sha256.get(..16) == Some(name.as_str()))
            .ok_or_else(|| anyhow::anyhow!("UnsafeToResume: child study is not planned"))?
            .clone();
        let child: ResearchIntentV1 = serde_json::from_slice(
            &crate::autotune::research::report_store::read_private(&dir, "intent.json", uid)?,
        )?;
        anyhow::ensure!(
            crate::autotune::research::child_verify::derived_from(&child, parent, &name),
            "UnsafeToResume: a child study intent is not derived from the campaign's"
        );
        let open = |sub: &str| -> anyhow::Result<File> {
            let sub_dir = File::from(crate::syscall::openat_dir_nofollow(dir.as_fd(), sub)?);
            let meta = sub_dir.metadata()?;
            anyhow::ensure!(
                meta.uid() == uid && meta.mode() & 0o077 == 0,
                "untrusted child study directory"
            );
            Ok(sub_dir)
        };
        let (journal_dir, dispatch, evidence) =
            (open("journal")?, open("dispatch")?, open("evidence")?);
        if terminal.is_some() || parent_has_attempts {
            for dir in [&journal_dir, &dispatch, &evidence] {
                let mut entries = std::fs::read_dir(format!("/proc/self/fd/{}", dir.as_raw_fd()))?;
                anyhow::ensure!(
                    entries.next().transpose()?.is_none(),
                    "UnsafeToResume: attempt follows a terminal or parent history"
                );
            }
        }
        let schedule = build_schedule(&child)?;
        let len = schedule.len();
        let (journal, adopted) =
            ResearchJournal::open_for_recovery(journal_dir.into(), uid, schedule)?;
        let witnesses = factory(&child)?;
        let mut driver = Driver::new(journal, child, dispatch, evidence, len);
        if let Some(slot) = adopted {
            driver.recover_interrupted(slot, same_boot, witnesses, now)?;
        }
        let records = driver.records()?;
        any_attempts |= !records.is_empty();
        if let Some(class) = records
            .last()
            .and_then(|record| record.outcome.terminal_failure())
        {
            terminal = Some(class);
        }
        recovered.push(RecoveredChild {
            dir,
            study,
            report: driver.finalize()?,
        });
    }
    Ok((terminal, any_attempts, recovered))
}

/// Once all owners and profiles are settled, keep each recovered child's independently rebuilt
/// report and record the observed sealed count. Missing budget and telemetry stay unknown.
fn record_recovered_children(
    campaign: &File,
    uid: u32,
    parent: &ResearchIntentV1,
    children: Vec<RecoveredChild>,
    allow_new: bool,
) -> anyhow::Result<()> {
    use crate::autotune::research::{
        child_ledger::open_ledger,
        child_study::StudyResult,
        report_store::{read_private, write_new},
    };
    if children.is_empty() {
        return Ok(());
    }
    let mut ledger = open_ledger(campaign, uid, parent, allow_new)?;
    for child in children {
        let bytes = serde_json::to_vec(&child.report)?;
        let existing = ledger
            .dispositions()
            .into_iter()
            .find(|item| item.study_sha256 == child.study.study_sha256)
            .ok_or_else(|| anyhow::anyhow!("UnsafeToResume: recovered study is not planned"))?;
        anyhow::ensure!(
            !matches!(
                existing.result,
                Some(StudyResult::Blocked | StudyResult::DeferredBudget)
            ),
            "UnsafeToResume: unexecuted decision has a child directory"
        );
        anyhow::ensure!(
            allow_new || existing.result.is_some(),
            "UnsafeToResume: terminal campaign lacks a child decision"
        );
        if !allow_new {
            anyhow::ensure!(
                read_private(&child.dir, "report.json", uid)? == bytes,
                "UnsafeToResume: terminal child report differs"
            );
        } else {
            match write_new(&child.dir, "report.json", &bytes) {
                Ok(()) => {}
                Err(error)
                    if error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|io| io.kind() == std::io::ErrorKind::AlreadyExists) =>
                {
                    anyhow::ensure!(
                        read_private(&child.dir, "report.json", uid)? == bytes,
                        "UnsafeToResume: recovered child report differs"
                    );
                }
                Err(error) => return Err(error),
            }
        }
        let count = u32::try_from(child.report.attempts_sealed)?;
        match ledger.attempts_for(&child.study.study_sha256) {
            Some(recorded) => anyhow::ensure!(
                recorded == count,
                "UnsafeToResume: recovered child attempt count differs"
            ),
            None => {
                ledger.record_run(&child.study, StudyResult::Failed, "InterruptedStudy", count)?
            }
        }
    }
    Ok(())
}

/// Put back the settings of any graphics profile a crash left applied, once every attempt is
/// settled (never before: settings are not restored under an unresolved game). An arm whose settings
/// file does not exist has nothing to restore; one that exists but cannot be trusted is unsafe.
fn restore_profiles(study_path: Option<&Path>, intent: &ResearchIntentV1) -> anyhow::Result<()> {
    let Some(study) = study_path else {
        return Ok(());
    };
    for tool in &intent.tools {
        restore_arm(study, tool.source, intent.uid)?;
    }
    Ok(())
}

/// One arm's graphics settings. Without a settings file there is nothing to edit-and-restore, but a
/// durable snapshot can survive without it (the file was removed after the edit): that is an
/// unrestored edit nobody can inspect, so recovery refuses rather than skipping the arm.
fn restore_arm(
    study: &Path,
    source: crate::autotune::research::ProtonSource,
    uid: u32,
) -> anyhow::Result<()> {
    use crate::autotune::research::{
        profile_applier::{StudySettingsFile, pending_edit, recover_guarded},
        session_host::arm_prefix_paths,
    };
    let settings = arm_prefix_paths(study, source).2;
    match std::fs::symlink_metadata(&settings) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            anyhow::ensure!(
                !pending_edit(&settings),
                "UnsafeToResume: a settings snapshot or marker survives without its settings file"
            );
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    }
    let file = StudySettingsFile::new(study, settings, uid)
        .map_err(|_| anyhow::anyhow!("UnsafeToResume: arm settings are not trustworthy"))?;
    recover_guarded(
        &file,
        &crate::autotune::research::profile_hook::cyberpunk_running,
    )
    .map_err(|_| anyhow::anyhow!("UnsafeToResume: a profile could not be restored"))?;
    Ok(())
}

#[cfg(test)]
#[path = "research_recovery_arm_tests.rs"]
mod arm_tests;

/// Retire the one interrupted attempt of a launched campaign and block the campaign. The attempt
/// is never replayed and the remaining attempts are not resumed: the consumed active time is not
/// recoverable. Any witness that cannot be produced or does not agree leaves the campaign unsafe.
fn recover_launched(
    lease: &mut CampaignLease,
    study_path: Option<&Path>,
    factory: &RecoveryFactory,
    now: u64,
) -> anyhow::Result<Option<&'static str>> {
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
    // The child studies of a funnel campaign are settled first, each through the same independent
    // witnesses, before the campaign may be retired.
    let parent_has_attempts = adopted.is_some() || !journal.verify()?.is_empty();
    let (child_terminal, child_attempts, recovered_children) = recover_children(
        &campaign,
        uid,
        &intent,
        factory,
        same_boot,
        now,
        parent_has_attempts,
    )?;
    let mut driver = Driver::new(journal, intent.clone(), dispatch, evidence, len);
    if let Some(slot) = adopted {
        driver.recover_interrupted(slot, same_boot, witnesses, now)?;
    }
    let parent_records = driver.records()?;
    anyhow::ensure!(
        !(child_attempts && !parent_records.is_empty()),
        "UnsafeToResume: parent and child attempt histories overlap"
    );
    let terminal = parent_records
        .last()
        .and_then(|record| record.outcome.terminal_failure())
        .or(child_terminal);
    // Both child and parent launch ownership must be settled before any settings are restored.
    restore_profiles(study_path, &intent)?;
    let allow_new = !matches!(
        lease.phase(),
        ResearchPhase::Completed | ResearchPhase::Blocked | ResearchPhase::Aborted
    );
    record_recovered_children(&campaign, uid, &intent, recovered_children, allow_new)?;
    if lease.phase() == ResearchPhase::Restoring
        && has_prepared_finalization(&campaign, uid, lease.intent_sha256())?
        && terminal.is_none()
    {
        return Ok(None);
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
        let phase = if terminal == Some(crate::autotune::research::driver::FailureClass::Integrity)
        {
            ResearchPhase::Aborted
        } else {
            ResearchPhase::Blocked
        };
        lease.advance(phase, now)?;
    } else if let Some(class) = terminal {
        let expected = if class == crate::autotune::research::driver::FailureClass::Integrity {
            ResearchPhase::Aborted
        } else {
            ResearchPhase::Blocked
        };
        anyhow::ensure!(
            lease.phase() == expected,
            "UnsafeToResume: terminal lease contradicts the recorded failure"
        );
    }
    Ok(terminal.map(|class| class.terminal_code()))
}
