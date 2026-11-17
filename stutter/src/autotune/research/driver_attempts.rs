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
            running: None,
            last_step: None,
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

    /// Keep every attempt's telemetry receipt beside the study (write-once; a conflicting file means
    /// the receipts cannot be recorded and the study simply has none).
    fn record_telemetry(&self, study: &ChildStudy) -> anyhow::Result<()> {
        let Some(telemetry) = &self.telemetry else {
            return Ok(());
        };
        let receipts = telemetry
            .lock()
            .map_err(|_| anyhow::anyhow!("the telemetry observer is poisoned"))?
            .take_receipts();
        if receipts.is_empty() {
            return Ok(());
        }
        // A receipt that cannot be made durable is an evidence failure, not a quiet omission.
        let studies = open_dir(&self.campaign, STUDIES_DIR)?;
        let dir = open_dir(&studies, dir_name(study))?;
        write_new(
            &dir,
            "telemetry-receipts.json",
            &serde_json::to_vec_pretty(&receipts)?,
        )
        .map_err(|error| anyhow::anyhow!("telemetry receipts not recorded: {error:#}"))
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
        super::campaign_budget::record(&dir, &evidence, self.parent.uid, driver.active_seconds())
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

    fn begin(&mut self, study: &ChildStudy) -> anyhow::Result<()> {
        let inputs = (self.select_arms)(study)?;
        let mut intent = child_intent(&self.parent, study, &inputs.arms, inputs.rule.as_ref())?;
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
        let driver = Driver::new(journal, intent, dispatch, evidence, len).with_prior(prior);
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
        self.record_telemetry(study)?;
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
            activation: self.feature_activation(study, u32::try_from(report.attempts_completed)?),
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
        self.observe_feature(study);
        self.arm_telemetry();
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
                // What this child used is recorded before anything else is decided about it, so
                // the next child starts from the campaign's true spend; unrecorded fails closed.
                self.record_budget(study, &driver)?;
                Ok(Some(self.outcome(study, &driver, halted)?))
            }
        }
    }
}

#[cfg(test)]
#[path = "driver_attempts_tests.rs"]
mod tests;
