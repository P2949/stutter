//! The real `StudyAttempts`: one child study run through the normal journalled `Driver`, polled one
//! tick at a time (PLAN section 1 nesting). The child gets its own intent (`child_intent`), and its
//! own journal, dispatch and evidence directories under `studies/<digest>` in the campaign, so a
//! study's attempts are sealed exactly like a whole campaign's and never mix with a sibling's. The
//! study's result is derived only from what its own sealed records prove.

use std::{collections::BTreeMap, fs::File, os::fd::AsFd, path::PathBuf};

use super::{
    ProtonSource, ResearchIntentV1, ResearchJournal,
    action_child::ActionChildScope,
    action_gate::SharedActionGate,
    child_intent::child_intent,
    child_study::{ChildStudy, Stage},
    driver::{AttemptHost, Driver, Step, build_schedule},
    loaded_runtime::RuntimeBinding,
    native_context::ProfileConformance,
    pilot_rule::FrozenConfirmationRule,
    raw_verify::verify_raw_evidence,
    report::evidence_context_consistent,
    report_store::{read_evidence, write_new, write_report},
    study_executor::{AttemptsOutcome, StudyAttempts},
};

/// What a child study runs with: the arms it runs across, and (for a confirmation only) the rule its
/// pilot froze. Both come from the funnel's earlier, recorded results, never from the study itself.
pub struct StudyInputs {
    pub arms: Vec<ProtonSource>,
    pub rule: Option<FrozenConfirmationRule>,
}

/// Chooses a child study's inputs.
pub type SelectArms = Box<dyn FnMut(&ChildStudy) -> anyhow::Result<StudyInputs> + Send>;

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
    /// The driver step of the last poll, for the controller to advance the lease from.
    last_step: Option<Step>,
    /// The controller's own `now` for the next tick, overriding the clock (one tick only).
    override_now: Option<u64>,
    /// The action gate an action child arms on the sighted game process (absent: never armed).
    gate: Option<SharedActionGate>,
    armed_for: Option<String>,
    /// The running study is a within-source treatment study (the gate then permits an action only
    /// during attempts treated On).
    treatment_study: bool,
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
            last_step: None,
            override_now: None,
            gate: None,
            armed_for: None,
            treatment_study: false,
        }
    }

    /// Share the action gate with the runtime: an Action study arms it on the game process it sights.
    pub fn with_action_gate(mut self, gate: SharedActionGate) -> Self {
        self.gate = Some(gate);
        self
    }

    /// The controller's time for the next tick: the campaign clock is the controller's, not ours.
    pub fn set_now(&mut self, now: u64) {
        self.override_now = Some(now);
    }

    /// The shared attempt host, for the controller to feed its desktop observations.
    pub fn host_mut(&mut self) -> &mut (dyn AttemptHost + Send) {
        self.host.as_mut()
    }

    /// An Action study arms the gate (once) on the game process the host has sighted and attested,
    /// authorizing only that study's action, for the rest of the campaign's window.
    fn arm_gate_for(&mut self, study: &ChildStudy) {
        let (Some(gate), Stage::Action(action)) = (&self.gate, study.stage) else {
            return;
        };
        if self.armed_for.as_deref() != Some(study.study_sha256.as_str())
            && let Some(target) = self.host.game_process()
        {
            let scope = ActionChildScope::new([action], target, self.parent.deadline_unix_seconds);
            if gate.lock().is_ok_and(|mut gate| gate.arm(scope)) {
                self.armed_for = Some(study.study_sha256.clone());
            }
        }
        // In a treatment study the action may be applied only while an attempt treated On runs; this
        // is set after arming, which starts from permitting.
        if self.treatment_study {
            let on = self.host.attempt_treatment() == Some(super::Treatment::On);
            if let Ok(mut gate) = gate.lock() {
                gate.set_attempt_on(on);
            }
        }
    }

    /// What the gate saw during an Action study: an applied action, decided, with its restoration
    /// proven. Anything less (nothing applied, or an action still outstanding) is not evidence.
    fn stage_evidence(&self, study: &ChildStudy) -> Option<bool> {
        let (Some(gate), Stage::Action(_)) = (&self.gate, study.stage) else {
            return None;
        };
        let evidence = gate.lock().ok()?.disarm();
        // The receipts are kept beside the study so an independent verification can read them.
        if !evidence.receipts.is_empty()
            && let Ok(studies) = open_dir(&self.campaign, STUDIES_DIR)
            && let Ok(dir) = open_dir(&studies, dir_name(study))
            && let Ok(bytes) = serde_json::to_vec_pretty(&evidence.receipts_json())
        {
            let _ = write_new(&dir, "action-receipts.json", &bytes);
        }
        Some(evidence.clean && !evidence.settled.is_empty() && evidence.kernel_state_proven)
    }

    #[cfg(test)]
    fn has_running(&self) -> bool {
        self.running.is_some()
    }

    fn begin(&mut self, study: &ChildStudy) -> anyhow::Result<()> {
        let inputs = (self.select_arms)(study)?;
        let intent = child_intent(&self.parent, study, &inputs.arms, inputs.rule.as_ref())?;
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
        // The child's own frozen intent, so a verifier can rebuild its schedule and report exactly as
        // it does for a whole campaign (`report_store::load_and_verify` on this directory).
        write_new(&study_dir, "intent.json", &serde_json::to_vec(&intent)?)?;
        // The host enables exactly this study's factor (none for any other stage), so a later study
        // never inherits an earlier one's environment.
        let factors = match study.stage {
            Stage::Feature(factor) => vec![factor],
            _ => Vec::new(),
        };
        self.host.set_enabled_factors(&factors);
        let treatments = super::treatment::treatments_of(&intent)?;
        self.treatment_study = treatments.is_some();
        self.host.set_treatments(treatments);
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
        let study_dir = open_dir(&open_dir(&self.campaign, STUDIES_DIR)?, dir_name(study))?;
        // The study's report is sealed beside its journal; verification recomputes and compares it.
        write_report(&study_dir, &report)?;
        let evidence_dir = open_dir(&study_dir, "evidence")?;
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
            // No runtime proof of a factor's activation, and no worker restoration evidence for an
            // action, is collected here yet: stage evidence is unproven.
            stage_evidence: self.stage_evidence(study),
            run_fps: evidence
                .values()
                .filter_map(|item| item.native_completion.as_deref())
                .map(|native| (native.proton_source, native.metrics.average_fps))
                .collect(),
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
        // The attempt is released only once the host proves it settled; otherwise the driver that owns
        // it stays, so the unsettled attempt remains visible to the next tick and to recovery.
        self.host.abort().map_err(|_| "SettlementUnproven")?;
        self.running = None;
        Ok(())
    }

    fn settle_interrupted(&mut self, _study: &ChildStudy) -> Result<(), &'static str> {
        self.host.abort().map_err(|_| "SettlementUnproven")
    }

    fn take_step(&mut self) -> Option<Step> {
        self.last_step.take()
    }

    fn poll(
        &mut self,
        study: &ChildStudy,
        remaining_attempts: u32,
    ) -> anyhow::Result<Option<AttemptsOutcome>> {
        self.poll_fenced(study, remaining_attempts, &mut || Ok(()))
    }

    fn poll_fenced(
        &mut self,
        study: &ChildStudy,
        _remaining_attempts: u32,
        fence: &mut dyn FnMut() -> anyhow::Result<()>,
    ) -> anyhow::Result<Option<AttemptsOutcome>> {
        if self.running.is_none() {
            self.begin(study)?;
        }
        let now = self.override_now.take().unwrap_or_else(|| (self.clock)());
        let own_fence = &mut self.launch_fence;
        let step = match self.running.as_mut() {
            Some((_, driver)) => driver.tick_with_launch_fence(self.host.as_mut(), now, || {
                own_fence()?;
                fence()
            })?,
            None => return Ok(None),
        };
        self.last_step = Some(step.clone());
        self.arm_gate_for(study);
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
