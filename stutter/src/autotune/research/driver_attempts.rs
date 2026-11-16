//! The real `StudyAttempts`: one child study run through the normal journalled `Driver`, polled one
//! tick at a time (PLAN section 1 nesting). The child gets its own intent (`child_intent`), and its
//! own journal, dispatch and evidence directories under `studies/<digest>` in the campaign, so a
//! study's attempts are sealed exactly like a whole campaign's and never mix with a sibling's. The
//! study's result is derived only from what its own sealed records prove.

use std::{collections::BTreeMap, fs::File, os::fd::AsFd, path::PathBuf};

use super::{
    ProtonSource, ResearchIntentV1, ResearchJournal,
    child_intent::child_intent,
    child_study::ChildStudy,
    driver::{AttemptHost, Driver, Step, build_schedule},
    loaded_runtime::RuntimeBinding,
    native_context::ProfileConformance,
    raw_verify::verify_raw_evidence,
    report::evidence_context_consistent,
    report_store::read_evidence,
    study_executor::{AttemptsOutcome, StudyAttempts},
};

/// Chooses the arms a child study runs across.
pub type SelectArms = Box<dyn FnMut(&ChildStudy) -> anyhow::Result<Vec<ProtonSource>> + Send>;

const STUDIES_DIR: &str = "studies";

pub struct DriverStudyAttempts {
    campaign: File,
    parent: ResearchIntentV1,
    host: Box<dyn AttemptHost + Send>,
    clock: Box<dyn FnMut() -> u64 + Send>,
    /// Called before a launch is armed: the controller records the campaign lease's launching phase.
    launch_fence: Box<dyn FnMut() -> anyhow::Result<()> + Send>,
    /// Where each arm's native results live, to replay them against their sealed records.
    results_for: Box<dyn Fn(ProtonSource) -> PathBuf + Send>,
    /// The arms a study runs across: every authorized tool for a screen, and the two arms chosen from
    /// the screen's results for a confirmation, feature or action study.
    select_arms: SelectArms,
    running: Option<(String, Driver)>,
}

fn dir_name(study: &ChildStudy) -> &str {
    &study.study_sha256[..16]
}

fn create_dir(parent: &File, name: &str) -> anyhow::Result<File> {
    crate::syscall::mkdir_private_at(parent.as_fd(), name)?;
    Ok(File::from(crate::syscall::openat_dir_nofollow(
        parent.as_fd(),
        name,
    )?))
}

fn open_dir(parent: &File, name: &str) -> std::io::Result<File> {
    crate::syscall::openat_dir_nofollow(parent.as_fd(), name).map(File::from)
}

impl DriverStudyAttempts {
    pub fn new(
        campaign: File,
        parent: ResearchIntentV1,
        host: Box<dyn AttemptHost + Send>,
        clock: Box<dyn FnMut() -> u64 + Send>,
        launch_fence: Box<dyn FnMut() -> anyhow::Result<()> + Send>,
        results_for: Box<dyn Fn(ProtonSource) -> PathBuf + Send>,
        select_arms: SelectArms,
    ) -> Self {
        Self {
            campaign,
            parent,
            host,
            clock,
            launch_fence,
            results_for,
            select_arms,
            running: None,
        }
    }

    fn begin(&mut self, study: &ChildStudy) -> anyhow::Result<()> {
        let arms = (self.select_arms)(study)?;
        let intent = child_intent(&self.parent, study, &arms)?;
        let studies = match open_dir(&self.campaign, STUDIES_DIR) {
            Ok(dir) => dir,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                create_dir(&self.campaign, STUDIES_DIR)?
            }
            Err(error) => return Err(error.into()),
        };
        // A directory that already exists means the study ran before: it is adopted by nobody here
        // (the executor refuses to start such a study), so creating it again fails closed.
        let study_dir = create_dir(&studies, dir_name(study))?;
        let journal_dir = create_dir(&study_dir, "journal")?;
        let dispatch = create_dir(&study_dir, "dispatch")?;
        let evidence = create_dir(&study_dir, "evidence")?;
        let schedule = build_schedule(&intent)?;
        let len = schedule.len();
        let journal = ResearchJournal::open(journal_dir.into(), intent.uid, schedule)?;
        let driver = Driver::new(journal, intent, dispatch, evidence, len);
        self.running = Some((study.study_sha256.clone(), driver));
        Ok(())
    }

    fn outcome(
        &self,
        study: &ChildStudy,
        driver: &Driver,
        health_halt: bool,
    ) -> anyhow::Result<AttemptsOutcome> {
        let report = driver.finalize()?;
        let evidence_dir = open_dir(&open_dir(&self.campaign, STUDIES_DIR)?, dir_name(study))
            .and_then(|dir| open_dir(&dir, "evidence"))?;
        let evidence: BTreeMap<_, _> = read_evidence(&evidence_dir, self.parent.uid)?;
        let conformance = evidence
            .values()
            .map(|item| {
                item.native_completion
                    .as_deref()
                    .and_then(|native| native.context.as_ref())
                    .map_or(ProfileConformance::Unproven, |context| {
                        context.conformance(study.profile)
                    })
            })
            .collect();
        // Replay every completed attempt's native files against its sealed record, and require every
        // completed result to carry a matched runtime.
        let raw = verify_raw_evidence(
            &evidence_dir,
            self.parent.uid,
            &driver.records()?,
            &*self.results_for,
        )?;
        let runtime_matched = !evidence.is_empty()
            && evidence.values().all(|item| {
                item.native_completion
                    .as_deref()
                    .is_some_and(|native| native.runtime_binding == Some(RuntimeBinding::Matched))
            });
        Ok(AttemptsOutcome {
            attempts_used: u32::try_from(report.attempts_sealed)?,
            completed: u32::try_from(report.attempts_completed)?,
            health_halt,
            context_consistent: evidence_context_consistent(&evidence),
            conformance,
            raw_verified: raw.passed(),
            runtime_matched,
        })
    }
}

impl StudyAttempts for DriverStudyAttempts {
    fn has_progress(&mut self, study: &ChildStudy) -> bool {
        match open_dir(&self.campaign, STUDIES_DIR) {
            Ok(studies) => open_dir(&studies, dir_name(study)).is_ok(),
            Err(error) => error.kind() != std::io::ErrorKind::NotFound,
        }
    }

    fn abort(&mut self) -> Result<(), &'static str> {
        // The host stops only what this campaign owns and verifies the settlement; the child's
        // journal keeps any unsealed attempt, which a restart finds and refuses to replay.
        self.running = None;
        self.host.abort().map_err(|_| "SettlementUnproven")
    }

    fn settle_interrupted(&mut self, _study: &ChildStudy) -> Result<(), &'static str> {
        self.host.abort().map_err(|_| "SettlementUnproven")
    }

    fn poll(
        &mut self,
        study: &ChildStudy,
        _remaining_attempts: u32,
    ) -> anyhow::Result<Option<AttemptsOutcome>> {
        if self.running.is_none() {
            self.begin(study)?;
        }
        let now = (self.clock)();
        let fence = &mut self.launch_fence;
        let step = match self.running.as_mut() {
            Some((_, driver)) => driver.tick_with_launch_fence(self.host.as_mut(), now, fence)?,
            None => return Ok(None),
        };
        let halted = matches!(step, Step::Halted(_));
        match step {
            Step::Waiting | Step::Running(_) | Step::Sealed(_) => Ok(None),
            Step::Done
            | Step::Expired
            | Step::ActiveBudgetExhausted
            | Step::Cancelled
            | Step::Halted(_) => {
                let Some((_, driver)) = self.running.take() else {
                    return Ok(None);
                };
                Ok(Some(self.outcome(study, &driver, halted)?))
            }
        }
    }
}

#[cfg(test)]
#[path = "driver_attempts_tests.rs"]
mod tests;
