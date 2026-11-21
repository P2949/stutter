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
    report::{AttemptEvidenceV1, evidence_context_consistent},
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

/// Consecutive failed attempts to record a settled child before it is treated as unresolved.
const MAX_PERSIST_RETRIES: u32 = 3;
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
    claim_verifier: Option<Box<dyn Fn() -> Box<dyn super::driver::ClaimSettlementVerifier> + Send>>,
    running: Option<(String, Driver)>,
    /// The driver step of the last poll, for the controller to advance the lease from.
    last_step: Option<Step>,
    /// The attempt has settled and its end (halted, terminal failure) is only awaiting durable
    /// accounting; the driver is never ticked again, only the persistence is retried.
    pending_end: Option<(bool, Option<super::driver::FailureClass>)>,
    persist_failures: u32,
    /// The controller's own `now` for the next tick, overriding the clock (one tick only).
    override_now: Option<u64>,
    /// The action gate an action child arms on the sighted game process (absent: never armed).
    gate: Option<SharedActionGate>,
    /// What the resident runtime reports of the game during each attempt (absent: nothing kept).
    telemetry: Option<super::telemetry_observer::SharedTelemetry>,
    armed_for: Option<String>,
    /// The game process the gate is currently scoped to.
    armed_target: Option<super::action_child::ChildTarget>,
    /// The running study is a within-source treatment study (the gate then permits an action only
    /// during attempts treated On).
    treatment_study: bool,
    /// Reads a process's environment (`/proc/<pid>/environ`); replaceable so the evidence can be tested.
    environ_of: Box<dyn Fn(u32) -> Option<Vec<u8>> + Send>,
    /// The build a feature study measures, and for each attempt whether its factor's treatment was
    /// observed in the game process's environment (index -> (treatment, reached)).
    feature_source: Option<ProtonSource>,
    feature_observed: BTreeMap<u32, (super::Treatment, bool)>,
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
            claim_verifier: None,
            running: None,
            last_step: None,
            pending_end: None,
            persist_failures: 0,
            override_now: None,
            gate: None,
            telemetry: None,
            armed_for: None,
            armed_target: None,
            treatment_study: false,
            environ_of: Box::new(|pid| std::fs::read(format!("/proc/{pid}/environ")).ok()),
            feature_source: None,
            feature_observed: BTreeMap::new(),
        }
    }

    pub(crate) fn with_claim_verifier_factory(
        mut self,
        factory: Box<dyn Fn() -> Box<dyn super::driver::ClaimSettlementVerifier> + Send>,
    ) -> Self {
        self.claim_verifier = Some(factory);
        self
    }

    /// Share the telemetry observer with the runtime: each attempt's receipt is kept beside its study.
    pub fn with_telemetry(mut self, telemetry: super::telemetry_observer::SharedTelemetry) -> Self {
        self.telemetry = Some(telemetry);
        self
    }

    /// Share the action gate with the runtime: an Action study arms it on the game process it sights.
    pub fn with_action_gate(mut self, gate: SharedActionGate) -> Self {
        self.gate = Some(gate);
        self
    }

    /// Replace how a game process's environment is read.
    #[cfg(test)]
    fn with_environ_reader(mut self, reader: Box<dyn Fn(u32) -> Option<Vec<u8>> + Send>) -> Self {
        self.environ_of = reader;
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

    /// Observe the attested game process of the running attempt: the observer counts what the resident
    /// runtime reports of it from here on.
    fn arm_telemetry(&mut self) {
        let (Some(telemetry), Some(target)) = (&self.telemetry, self.host.game_process()) else {
            return;
        };
        let attempt = self.host.attempt_index();
        if let Ok(mut observer) = telemetry.lock() {
            observer.arm(target, attempt);
        }
    }

    /// During a feature study, once per attempt: does the factor's treatment (on or off) show in the
    /// sighted game process's own environment? Observation only; an unreadable process records nothing.
    fn observe_feature(&mut self, study: &ChildStudy) {
        let Stage::Feature(factor) = study.stage else {
            return;
        };
        let (Some(source), Some(target)) = (self.feature_source, self.host.game_process()) else {
            return;
        };
        let (Some(treatment), Some(index)) =
            (self.host.attempt_treatment(), self.host.attempt_index())
        else {
            return;
        };
        if self.feature_observed.contains_key(&index) {
            return;
        }
        let Some(environ) = (self.environ_of)(u32::from(target.pid)) else {
            return;
        };
        let reached = super::features::treatment_effective(
            &environ,
            source,
            factor,
            treatment == super::Treatment::On,
        );
        self.feature_observed.insert(index, (treatment, reached));
    }

    /// What the game itself reported of a feature study's factor across its completed attempts (frame
    /// generation and the upscaler appear in the native context); `None` for any other study.
    fn feature_functional(
        &self,
        study: &ChildStudy,
        evidence: &BTreeMap<u32, AttemptEvidenceV1>,
    ) -> Option<super::functional_probe::Functional> {
        let Stage::Feature(factor) = study.stage else {
            return None;
        };
        let mut attempts = Vec::new();
        for (slot, item) in evidence {
            // Which treatment an attempt ran is known only where it was observed.
            let Some((treatment, _)) = self.feature_observed.get(slot) else {
                return Some(super::functional_probe::Functional::Unproven);
            };
            attempts.push((
                *treatment,
                item.native_completion
                    .as_deref()
                    .and_then(|native| native.context.as_ref()),
            ));
        }
        Some(super::functional_probe::probe(factor, &attempts))
    }

    /// Whether every completed attempt of a feature study showed its own treatment in the game
    /// process's environment (and both treatments ran). `None` for any other study.
    fn feature_activation(&self, study: &ChildStudy, completed: u32) -> Option<bool> {
        matches!(study.stage, Stage::Feature(_)).then(|| {
            let both = [super::Treatment::Off, super::Treatment::On]
                .iter()
                .all(|wanted| self.feature_observed.values().any(|(t, _)| t == wanted));
            both && self.feature_observed.len() as u32 >= completed
                && self.feature_observed.values().all(|(_, reached)| *reached)
        })
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
                self.armed_target = Some(target);
            }
        } else if let Some(target) = self.host.game_process()
            && self.armed_target != Some(target)
            && gate.lock().is_ok_and(|mut gate| gate.rebind(target))
        {
            // A later attempt is a new game process: the action is scoped to that process alone,
            // never inherited from the previous launch.
            self.armed_target = Some(target);
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
        // The receipts must be durable beside the study, where an independent verification reads them:
        // a failed or conflicting write is a failed qualification, never a silent one.
        let durable = !evidence.receipts.is_empty()
            && self
                .write_receipts(study, &evidence.receipts_json())
                .is_ok();
        Some(
            evidence.clean
                && !evidence.settled.is_empty()
                && evidence.kernel_state_proven
                && durable,
        )
    }

    /// Record what a finished child spent of the campaign's active-time and evidence budgets.
    fn record_budget(&self, study: &ChildStudy, driver: &Driver) -> anyhow::Result<()> {
        let studies = open_dir(&self.campaign, STUDIES_DIR)?;
        let dir = open_dir(&studies, dir_name(study))?;
        let evidence = open_dir(&dir, "evidence")?;
        super::campaign_budget::record(
            &dir,
            &evidence,
            self.parent.uid,
            driver.child_active_seconds(),
        )
    }

    /// Record the action receipts in the study directory. An existing file must be byte-identical.
    fn write_receipts(
        &self,
        study: &ChildStudy,
        receipts: &serde_json::Value,
    ) -> anyhow::Result<()> {
        let studies = open_dir(&self.campaign, STUDIES_DIR)?;
        let dir = open_dir(&studies, dir_name(study))?;
        let bytes = serde_json::to_vec_pretty(receipts)?;
        match write_new(&dir, "action-receipts.json", &bytes) {
            Ok(()) => Ok(()),
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|io| io.kind() == std::io::ErrorKind::AlreadyExists) =>
            {
                anyhow::ensure!(
                    super::report_store::read_private(
                        &dir,
                        "action-receipts.json",
                        self.parent.uid
                    )? == bytes,
                    "the recorded action receipts differ"
                );
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    #[cfg(test)]
    fn has_running(&self) -> bool {
        self.running.is_some()
    }

    fn begin(&mut self, study: &ChildStudy, remaining_attempts: u32) -> anyhow::Result<()> {
        let inputs = (self.select_arms)(study)?;
        let mut intent = child_intent(&self.parent, study, &inputs.arms, inputs.rule.as_ref())?;
        anyhow::ensure!(
            intent.max_attempts <= remaining_attempts,
            "selected inputs exceed remaining campaign budget"
        );
        // The arms must run over comparable prefixes before anything is created for the study.
        let cohort = self
            .host
            .check_prefixes(&inputs.arms)
            .map_err(|code| anyhow::anyhow!("the study's prefixes are not comparable: {code}"))?;
        // The cohort is sealed into the child's own intent, so it is part of what the journal and the
        // report are bound to and cannot be rewritten afterwards.
        intent.prefix_cohort = cohort.map(|class| class.label().to_owned());
        let studies = match open_dir(&self.campaign, STUDIES_DIR) {
            Ok(dir) => dir,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                create_dir(&self.campaign, STUDIES_DIR)?
            }
            Err(error) => return Err(error.into()),
        };
        // What the campaign's earlier children already used, read before this one's directory exists.
        let prior = super::campaign_budget::spent(
            &studies,
            self.parent.uid,
            u64::from(self.parent.max_active_seconds),
        )?;
        // A directory that already exists means the study ran before: it is adopted by nobody here
        // (the executor refuses to start such a study), so creating it again fails closed.
        let study_dir = create_dir(&studies, dir_name(study))?;
        let journal_dir = create_dir(&study_dir, "journal")?;
        let dispatch = create_dir(&study_dir, "dispatch")?;
        let evidence = create_dir(&study_dir, "evidence")?;
        // The child's own frozen intent, so a verifier can rebuild its schedule and report exactly as
        // it does for a whole campaign (`report_store::load_and_verify` on this directory).
        write_new(&study_dir, "intent.json", &serde_json::to_vec(&intent)?)?;
        // The cache cohort the arms ran in (PLAN D6, section 6.1), recorded beside the study so a
        // cold and a warm study are never mistaken for one another in the bundle.
        if let Some(cohort) = cohort {
            let record = serde_json::json!({ "schema": 1, "cohort": cohort.label() });
            write_new(
                &study_dir,
                "prefix-cohort.json",
                &serde_json::to_vec(&record)?,
            )?;
        }
        // The host enables exactly this study's factor (none for any other stage), so a later study
        // never inherits an earlier one's environment.
        let factors = match study.stage {
            Stage::Feature(factor) => vec![factor],
            _ => Vec::new(),
        };
        self.host.set_enabled_factors(&factors);
        let treatments = super::treatment::treatments_of(&intent)?;
        self.treatment_study = treatments.is_some();
        self.feature_source = inputs.arms.first().copied();
        self.feature_observed.clear();
        self.host.set_treatments(treatments);
        let schedule = build_schedule(&intent)?;
        let len = schedule.len();
        let journal = ResearchJournal::open(journal_dir.into(), intent.uid, schedule)?;
        let mut driver = Driver::new(journal, intent, dispatch, evidence, len).with_prior(prior);
        if let Some(factory) = &self.claim_verifier {
            driver.set_claim_verifier(factory());
        }
        self.running = Some((study.study_sha256.clone(), driver));
        Ok(())
    }

    fn outcome(
        &self,
        study: &ChildStudy,
        driver: &Driver,
        health_halt: bool,
        terminal_failure: Option<super::driver::FailureClass>,
    ) -> anyhow::Result<AttemptsOutcome> {
        let report = driver.finalize()?;
        self.record_telemetry(study)?;
        let study_dir = open_dir(&open_dir(&self.campaign, STUDIES_DIR)?, dir_name(study))?;
        // The study's report is sealed beside its journal; verification recomputes and compares it.
        match write_report(&study_dir, &report) {
            Ok(()) => {}
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|io| io.kind() == std::io::ErrorKind::AlreadyExists) =>
            {
                anyhow::ensure!(
                    super::report_store::read_private(&study_dir, "report.json", self.parent.uid)?
                        == serde_json::to_vec(&report)?,
                    "the recorded child report differs"
                );
            }
            Err(error) => return Err(error),
        }
        let evidence_dir = open_dir(&study_dir, "evidence")?;
        let evidence: BTreeMap<_, _> = read_evidence(&evidence_dir, self.parent.uid)?;
        let conformance = evidence
            .values()
            .map(|item| {
                item.native_completion
                    .as_deref()
                    .and_then(|native| native.context.as_ref())
                    .map_or(ProfileConformance::Unproven, |context| {
                        context.conformance_in(study.profile, study.stage)
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
        let activation = self.feature_activation(study, u32::try_from(report.attempts_completed)?);
        let functional = self.feature_functional(study, &evidence);
        // The game's enum cannot establish the exact backend or functional fork-specific activation.
        // Keep feature studies unqualified until independently replayable identity and operation
        // evidence is connected, even when the environment and game-reported selection agree.
        let stage_evidence = match study.stage {
            Stage::Feature(_) => Some(false),
            _ => self.stage_evidence(study),
        };
        Ok(AttemptsOutcome {
            planned_attempts: driver.intent().max_attempts,
            source_order: driver
                .intent()
                .tools
                .iter()
                .map(|tool| tool.source)
                .collect(),
            attempts_used: u32::try_from(report.attempts_sealed)?,
            completed: u32::try_from(report.attempts_completed)?,
            failures: driver
                .records()?
                .into_iter()
                .filter_map(|record| record.outcome.compatibility_failure().map(str::to_owned))
                .collect(),
            health_halt,
            terminal_failure,
            context_consistent: evidence_context_consistent(
                &evidence,
                driver.intent().treatment.as_ref(),
            ),
            conformance,
            raw_verified: raw.passed(),
            runtime_matched,
            // No runtime proof of a factor's activation, and no worker restoration evidence for an
            // action, is collected here yet: stage evidence is unproven.
            stage_evidence,
            activation,
            functional,
            run_fps: evidence
                .values()
                .filter_map(|item| item.native_completion.as_deref())
                .map(|native| (native.proton_source, native.metrics.average_fps))
                .collect(),
        })
    }
}

impl StudyAttempts for DriverStudyAttempts {
    fn required_attempts(&mut self, study: &ChildStudy) -> anyhow::Result<u32> {
        let inputs = (self.select_arms)(study)?;
        if study.stage == Stage::Pilot {
            return u32::try_from(inputs.arms.len())?
                .checked_mul(3)
                .ok_or_else(|| anyhow::anyhow!("pilot attempt count overflow"));
        }
        Ok(study.min_attempts)
    }
    fn has_progress(&mut self, study: &ChildStudy) -> bool {
        match open_dir(&self.campaign, STUDIES_DIR) {
            Ok(studies) => open_dir(&studies, dir_name(study)).is_ok(),
            Err(error) => error.kind() != std::io::ErrorKind::NotFound,
        }
    }

    fn abort(&mut self) -> Result<(), &'static str> {
        // Cancellation goes through the owning Driver: it refuses an ambiguous launch and, for a
        // started attempt, retires tickets with independent witnesses before sealing the journal.
        // A host abort alone cannot prove that a sent launcher will not arrive later, nor settle a
        // consumed ticket. Keep the driver and profile whenever any part of that proof fails.
        if let Some((_, driver)) = self.running.as_mut() {
            driver.cancel();
            let now = self
                .override_now
                .take()
                .unwrap_or_else(|| (self.clock)())
                .max(driver.last_tick_seconds().unwrap_or(0));
            match driver.tick_with_launch_fence(self.host.as_mut(), now, || {
                anyhow::bail!("cancellation must not launch an attempt")
            }) {
                Ok(Step::Sealed(_)) | Ok(Step::Cancelled) | Ok(Step::Stopped(_))
                | Ok(Step::Halted(_)) => {}
                _ => return Err("SettlementUnproven"),
            }
        } else {
            self.host.abort().map_err(|_| "SettlementUnproven")?;
        }
        self.running = None;
        Ok(())
    }

    fn abort_outcome(
        &mut self,
        study: &ChildStudy,
    ) -> Result<Option<AttemptsOutcome>, &'static str> {
        let Some((digest, driver)) = self.running.as_mut() else {
            self.host.abort().map_err(|_| "SettlementUnproven")?;
            return Ok(None);
        };
        if digest != &study.study_sha256 {
            return Err("InterruptedStudyMismatch");
        }
        if self.pending_end.is_some() {
            // Already settled: only its accounting is outstanding, so settle nothing again.
            return self
                .persist_end(study)
                .map_err(|_| "InterruptedReportPersistenceFailed");
        }
        driver.cancel();
        let now = self
            .override_now
            .take()
            .unwrap_or_else(|| (self.clock)())
            .max(driver.last_tick_seconds().unwrap_or(0));
        let step = driver
            .tick_with_launch_fence(self.host.as_mut(), now, || {
                anyhow::bail!("cancellation must not launch an attempt")
            })
            .map_err(|_| "SettlementUnproven")?;
        if !matches!(
            step,
            Step::Sealed(_) | Step::Cancelled | Step::Stopped(_) | Step::Halted(_)
        ) {
            return Err("SettlementUnproven");
        }
        let (_, driver) = self.running.as_ref().ok_or("SettlementUnproven")?;
        self.record_budget(study, driver)
            .map_err(|_| "InterruptedBudgetPersistenceFailed")?;
        let terminal = match &step {
            Step::Stopped(class) => Some(*class),
            Step::Sealed(record) => record.outcome.terminal_failure(),
            _ => None,
        };
        if let Some(class) = terminal {
            self.last_step = Some(Step::Stopped(class));
        }
        let outcome = self
            .outcome(
                study,
                driver,
                terminal == Some(super::driver::FailureClass::Health),
                terminal,
            )
            .map_err(|_| "InterruptedReportPersistenceFailed")?;
        self.running = None;
        Ok(Some(outcome))
    }

    fn settle_interrupted(&mut self, _study: &ChildStudy) -> Result<(), &'static str> {
        // An in-memory host has no authority to reconstruct a prior driver's claimed tickets,
        // launcher state, worker tokens and process witnesses. The resident recovery job does
        // that against the child's durable intent and journal before restoring profiles.
        Err("SettlementUnproven")
    }

    fn persistence_pending(&self) -> bool {
        self.pending_end.is_some() && self.persist_failures < MAX_PERSIST_RETRIES
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
        remaining_attempts: u32,
        fence: &mut dyn FnMut() -> anyhow::Result<()>,
    ) -> anyhow::Result<Option<AttemptsOutcome>> {
        if self.pending_end.is_some() {
            // Settled and only awaiting durable accounting: retry that, never the launch machinery.
            return self.persist_end(study);
        }
        if self.running.is_none() {
            self.begin(study, remaining_attempts)?;
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
        self.observe_feature(study);
        self.arm_telemetry();
        let halted = matches!(step, Step::Halted(_));
        let terminal_failure = match step {
            Step::Stopped(class) => Some(class),
            _ => None,
        };
        match step {
            Step::Waiting | Step::Running(_) | Step::Sealed(_) => Ok(None),
            Step::OwnershipUnresolved(reason) => {
                anyhow::bail!("child ownership unresolved: {reason}")
            }
            Step::Done
            | Step::Expired
            | Step::ActiveBudgetExhausted
            | Step::Cancelled
            | Step::Halted(_)
            | Step::Stopped(_) => {
                self.pending_end = Some((halted, terminal_failure));
                self.persist_end(study)
            }
        }
    }
}

#[path = "driver_attempts_persistence.rs"]
mod persistence;

#[cfg(test)]
#[path = "driver_attempts_tests.rs"]
mod tests;
