//! The child-study funnel presented as a campaign driver. The resident controller drives one thing
//! per campaign and advances its lease from that thing's `Step`; this adapter lets the funnel be
//! that thing without changing any lease logic. Each tick polls the stage runner once and returns
//! the inner journalled driver's step (so the lease walks `Launching -> Sealing -> Analysing ->
//! NextArm -> AwaitSafeWindow` per attempt exactly as for a flat campaign), a campaign-level
//! termination (`Expired`, `ActiveBudgetExhausted`, `Cancelled`, `Halted`) when one occurs, and
//! `Done` only when every planned study has a recorded decision.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use super::{
    ResearchIntentV1,
    action_gate::SharedActionGate,
    arm_selection::{pilot_fps, pilot_of, screen_fps, screen_of, select_pair, viable_sources},
    child_ledger::{Disposition, open_for_campaign},
    child_runner::{RunnerStep, StageRunner},
    child_study::{ChildStudy, Stage, StudyArms},
    driver::{AttemptHost, Step},
    driver_attempts::{DriverStudyAttempts, SelectArms, StudyInputs},
    pilot_rule::FrozenConfirmationRule,
    study_executor::{HostStudyExecutor, ProfileHook},
    telemetry_observer::SharedTelemetry,
};

/// The rules qualified pilots froze, by pilot study digest: written from the ledger by the funnel,
/// read when a confirmation's inputs are chosen.
pub type RuleRegistry = Arc<Mutex<BTreeMap<String, FrozenConfirmationRule>>>;

/// The arms and rule of one child study, from its screen's measured FPS and the pilot's rule. A
/// screen runs across every authorized tool; every other stage across the control and the best
/// candidate; a confirmation also needs the rule its pilot froze.
pub fn inputs_for_study(
    parent: &ResearchIntentV1,
    study: &ChildStudy,
    fps: &BTreeMap<super::ProtonSource, Vec<f64>>,
    rule: Option<FrozenConfirmationRule>,
) -> anyhow::Result<StudyInputs> {
    if !matches!(study.stage, Stage::Init | Stage::Screen | Stage::Pilot) {
        anyhow::ensure!(
            fps.values()
                .all(|runs| runs.len() == super::pilot_rule::MIN_PILOT_RUNS),
            "selection requires three whole-run pilot measurements per viable build"
        );
    }
    Ok(match study.arms() {
        StudyArms::Listed => StudyInputs {
            arms: parent.prefix_init_arms.clone(),
            rule: None,
        },
        StudyArms::All => StudyInputs {
            arms: parent.tools.iter().map(|tool| tool.source).collect(),
            rule: None,
        },
        StudyArms::Viable => {
            let arms = viable_sources(parent, fps);
            if arms.len() < 2 {
                return Err(NoEligibleSource.into());
            }
            StudyInputs { arms, rule: None }
        }
        // A treatment study measures one build with the switch off and on: the stable control for an
        // action, and for a feature the best measured build that has an environment control for it.
        StudyArms::Single => StudyInputs {
            arms: vec![
                super::arm_selection::select_treatment_source(parent, study, fps)
                    .ok_or(NoEligibleSource)?,
            ],
            rule: None,
        },
        StudyArms::Pair => match study.stage {
            Stage::MatchedCachyOs => {
                let ca = parent
                    .tools
                    .iter()
                    .any(|tool| tool.source == super::ProtonSource::CachyOs);
                let c3 = parent
                    .tools
                    .iter()
                    .any(|tool| tool.source == super::ProtonSource::CachyOsV3);
                let ca_runs = fps.get(&super::ProtonSource::CachyOs);
                let c3_runs = fps.get(&super::ProtonSource::CachyOsV3);
                match (ca, c3, ca_runs, c3_runs) {
                    (true, true, Some(ca_runs), Some(c3_runs)) => {
                        let mut runs = Vec::new();
                        for &v in ca_runs {
                            runs.push((super::ProtonSource::CachyOs, v));
                        }
                        for &v in c3_runs {
                            runs.push((super::ProtonSource::CachyOsV3, v));
                        }
                        // The matched rule stands on the metric its pilot froze (the dependency's rule).
                        let matched_rule = super::pilot_rule::rule_from_runs(&runs)
                            .map_err(|e| anyhow::anyhow!("{e}"))?
                            .with_tail_metric(rule.as_ref().is_some_and(|rule| rule.tail_metric));
                        StudyInputs {
                            arms: vec![
                                super::ProtonSource::CachyOs,
                                super::ProtonSource::CachyOsV3,
                            ],
                            rule: Some(matched_rule),
                        }
                    }
                    _ => return Err(NoEligibleSource.into()),
                }
            }
            _ => StudyInputs {
                arms: select_pair(parent, fps).ok_or(NoEligibleSource)?.to_vec(),
                rule: match study.stage {
                    Stage::Confirm | Stage::Holdout => {
                        Some(rule.ok_or_else(|| anyhow::anyhow!("the pilot froze no rule"))?)
                    }
                    _ => None,
                },
            },
        },
    })
}

/// No measured viable pair or treatment source can run this study.
#[derive(Debug)]
pub struct NoEligibleSource;

impl std::fmt::Display for NoEligibleSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("no measured eligible source or comparison pair")
    }
}

impl std::error::Error for NoEligibleSource {}

/// Decide before a study starts whether its pair or treatment source can be selected at all, so a
/// study without viable inputs is blocked rather than halting the funnel or running on the wrong build.
/// A blocker that decides a study from the screen's results.
pub type SelectionBlocker = Box<dyn Fn(&ChildStudy) -> Option<&'static str> + Send>;

pub fn selection_blocker(
    campaign: std::fs::File,
    parent: ResearchIntentV1,
    rules: RuleRegistry,
) -> SelectionBlocker {
    let select = Mutex::new(production_inputs(campaign, parent, rules));
    Box::new(move |study| {
        if matches!(study.stage, Stage::Init | Stage::Screen) {
            return None;
        }
        let Ok(mut select) = select.lock() else {
            return Some("BlockedScreenUnavailable");
        };
        match select(study) {
            Ok(_) => None,
            Err(error) if error.downcast_ref::<NoEligibleSource>().is_some() => {
                Some("BlockedNoEligibleSource")
            }
            Err(error) => {
                let msg = error.to_string();
                if msg.contains("rebuild")
                    || msg.contains("tampered")
                    || msg.contains("PilotInvalid")
                    || msg.contains("evidence")
                    || msg.contains("corrupt")
                    || msg.contains("digest")
                {
                    Some("BlockedPredecessorCorrupt")
                } else if msg.contains("PilotTooThin") {
                    Some("BlockedPilotTooThin")
                } else {
                    Some("BlockedScreenUnavailable")
                }
            }
        }
    })
}

/// The production selector over a campaign's sealed screen evidence and the rule registry.
pub fn production_inputs(
    campaign: std::fs::File,
    parent: ResearchIntentV1,
    rules: RuleRegistry,
) -> SelectArms {
    Box::new(move |study| {
        let fps = match study.stage {
            Stage::Init | Stage::Screen => BTreeMap::new(),
            Stage::Pilot => screen_fps(&campaign, parent.uid, &screen_of(&parent, study)?)?,
            _ => pilot_fps(&campaign, parent.uid, &pilot_of(&parent, study)?)?,
        };
        let rule = study
            .depends_on
            .iter()
            .find_map(|digest| rules.lock().ok()?.get(digest).cloned());
        inputs_for_study(&parent, study, &fps, rule)
    })
}

pub struct FunnelDriver {
    runner: StageRunner,
    executor: HostStudyExecutor<Box<dyn ProfileHook + Send>, DriverStudyAttempts>,
    /// Studies this build cannot run are decided with their blocker, never run or dropped.
    blocker: fn(&ChildStudy) -> Option<&'static str>,
    /// Blockers that depend on the screen's own results (no viable pair or treatment source).
    selection: Option<SelectionBlocker>,
    cancelled: bool,
    rules: RuleRegistry,
    /// The action gate an action child arms, shared with the runtime (absent: no action child).
    gate: Option<SharedActionGate>,
    /// What the resident runtime sees of the game, shared with it (absent: none kept).
    telemetry: Option<SharedTelemetry>,
}

impl FunnelDriver {
    pub fn new(
        campaign: &std::fs::File,
        intent: &ResearchIntentV1,
        profiles: Box<dyn ProfileHook + Send>,
        attempts: DriverStudyAttempts,
        blocker: fn(&ChildStudy) -> Option<&'static str>,
        rules: RuleRegistry,
    ) -> anyhow::Result<Self> {
        let ledger = open_for_campaign(campaign, intent.uid, intent)?;
        Ok(Self {
            runner: StageRunner::new(ledger, intent.max_attempts),
            executor: HostStudyExecutor::new(profiles, attempts),
            blocker,
            selection: None,
            cancelled: false,
            rules,
            gate: None,
            telemetry: None,
        })
    }

    /// Add the blocker that decides, from the screen's results, which treatment studies can run.
    pub fn with_selection_blocker(mut self, selection: SelectionBlocker) -> Self {
        self.selection = Some(selection);
        self
    }

    /// Share the action gate the attempts arm, so the runtime can consult it.
    pub fn with_action_gate(mut self, gate: SharedActionGate) -> Self {
        self.gate = Some(gate);
        self
    }

    /// Share the observer whose receipts each attempt keeps.
    pub fn with_telemetry(mut self, telemetry: SharedTelemetry) -> Self {
        self.telemetry = Some(telemetry);
        self
    }

    pub fn telemetry(&self) -> Option<SharedTelemetry> {
        self.telemetry.clone()
    }

    pub fn action_gate(&self) -> Option<SharedActionGate> {
        self.gate.clone()
    }

    /// The shared attempt host, for the controller to feed its desktop observations.
    pub fn host_mut(&mut self) -> &mut (dyn AttemptHost + Send) {
        self.executor.attempts.host_mut()
    }

    #[cfg_attr(not(test), expect(dead_code, reason = "staged"))]
    pub fn dispositions(&self) -> Vec<Disposition> {
        self.runner.ledger().dispositions()
    }

    pub(crate) fn is_quiescent(&self) -> bool {
        self.executor.is_quiescent()
    }

    /// Request cancellation: the next tick settles the running attempt, restores the profile and
    /// ends the funnel without deciding what it prevented.
    pub fn cancel(&mut self) {
        self.cancelled = true;
    }

    /// Stop now (controller shutdown): settle, restore and prove it. `Err` is unproven.
    pub fn shutdown(&mut self) -> Result<(), &'static str> {
        self.runner
            .abort(&mut self.executor)
            .map_err(|_| "InterruptedDecisionPersistenceFailed")?
    }

    /// Take an urgent terminal cause observed while settling an interrupted child.
    pub(crate) fn take_terminal_failure(&mut self) -> Option<super::driver::FailureClass> {
        use super::child_runner::StudyExecutor;
        match self.executor.take_step() {
            Some(Step::Stopped(class)) => Some(class),
            Some(Step::Sealed(record)) => record.outcome.terminal_failure(),
            _ => None,
        }
    }

    /// One controller tick at the controller's own time, which also governs the child driver.
    pub fn tick_at(
        &mut self,
        now: u64,
        fence: &mut dyn FnMut() -> anyhow::Result<()>,
    ) -> anyhow::Result<Step> {
        self.executor.attempts.set_now(now);
        self.tick(fence)
    }

    /// One controller tick.
    pub fn tick(&mut self, fence: &mut dyn FnMut() -> anyhow::Result<()>) -> anyhow::Result<Step> {
        use super::child_runner::StudyExecutor;
        if self.cancelled {
            return Ok(match self.runner.abort(&mut self.executor)? {
                Ok(()) => match self.executor.take_step() {
                    Some(step @ (Step::Stopped(_) | Step::Halted(_))) => step,
                    _ => Step::Cancelled,
                },
                Err(reason) => Step::OwnershipUnresolved(reason),
            });
        }
        // Publish every rule the ledger holds, so a confirmation's inputs can find its pilot's.
        if let Ok(mut registry) = self.rules.lock() {
            for study in self.runner.ledger().dispositions() {
                if let Some(rule) = self.runner.ledger().rule_of(&study.study_sha256) {
                    registry.insert(study.study_sha256, rule.clone());
                }
            }
        }
        let blocked = self.executor.attempts.host_mut().health_blocked();
        // A study whose least possible time already exceeds the active seconds left is decided, not started: it
        // would only run into the campaign's budget stop. The ceiling (the attempt limit) is not used to refuse work.
        let facts = match self.executor.attempts.time_facts() {
            Ok(facts) => facts,
            // The campaign's own spend cannot be read: stop visibly rather than plan against a guess of zero.
            Err(code) => return Ok(Step::Halted(code)),
        };
        let (blocker, selection) = (self.blocker, self.selection.as_ref());
        let combined = |study: &ChildStudy| {
            blocker(study)
                .or_else(|| selection.and_then(|select| select(study)))
                .or_else(|| {
                    (super::child_study::time_feasibility(study, facts)
                        == super::child_study::TimeFeasibility::Impossible)
                        .then_some("DeferredActiveTime")
                })
        };
        let step =
            self.runner
                .step_fenced(&mut self.executor, &combined, &mut || blocked, fence)?;
        let inner = self.executor.take_step();
        Ok(match step {
            RunnerStep::Halted(code) => match inner {
                Some(Step::Stopped(class)) => Step::Stopped(class),
                _ if blocked => Step::Stopped(super::driver::FailureClass::Health),
                _ => Step::Halted(code),
            },
            RunnerStep::OwnershipUnresolved(code) => Step::OwnershipUnresolved(code),
            RunnerStep::Done => Step::Done,
            // While a study runs, and on the tick that finishes one, the inner driver's step moves
            // the lease. A campaign-level ending from inside a study ends the campaign; the
            // study's own completion (`Done`) is not the campaign's.
            RunnerStep::Pending | RunnerStep::Ran { .. } => match inner {
                Some(
                    ended @ (Step::Expired
                    | Step::ActiveBudgetExhausted
                    | Step::Cancelled
                    | Step::Halted(_)
                    | Step::Stopped(_)
                    | Step::OwnershipUnresolved(_)),
                ) => ended,
                Some(Step::Done) | None => Step::Waiting,
                Some(ongoing) => ongoing,
            },
        })
    }
}
