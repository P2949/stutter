//! The CP-LONG attempt host (PLAN section 7): an `AttemptHost` that launches a long attempt through the
//! ordinary host (same admission, tickets and settlement), plays the authorized work unit through a
//! `LongSessionDriver`, and finishes with a sealed session record instead of a native benchmark result.
//! Every other attempt is delegated untouched. The record is accepted only through
//! `long_receipt::verify`, which does not trust the driver's own claims, and the process tree is settled
//! through the inner host before the attempt can finish. With the build's `UnavailableDriver` a long
//! attempt fails with the typed `BlockedNoLongSessionDriver`; nothing is ever run on the benchmark.

use super::{
    AttemptSlot, ProtonSource, ResearchFactor, Treatment,
    action_child::ChildTarget,
    dispatch_context::{ArmSpec, DispatchContextV1},
    driver::{AttemptHost, FailureClass, Progress},
    driver_failure::LaunchFailure,
    long_receipt::{LongProgress, LongSessionDriver, LongSessionRecord, SessionProcess, verify},
    long_session::LongWorkUnit,
    report::AttemptEvidenceV1,
    steam_host::StopStep,
};

pub struct LongAttemptHost<H: AttemptHost, D: LongSessionDriver> {
    inner: H,
    driver: D,
    /// The long attempt in flight: its unit and the window it was authorized for.
    session: Option<(LongWorkUnit, u32)>,
    /// An outcome decided on an earlier poll whose game tree is still being stopped, one bounded step per poll.
    settling: Option<Outcome>,
}

/// What the attempt reports once its game is proven gone.
enum Outcome {
    Fail(FailureClass, String),
    Done {
        record: Box<LongSessionRecord>,
        unit: LongWorkUnit,
        window: u32,
        index: u32,
    },
}

impl<H: AttemptHost, D: LongSessionDriver> LongAttemptHost<H, D> {
    pub fn new(inner: H, driver: D) -> Self {
        Self {
            inner,
            driver,
            session: None,
            settling: None,
        }
    }

    /// An attempt of a child study on the long profile: it carries the authorized unit and runs under
    /// exactly the long window.
    fn long_of(context: &DispatchContextV1) -> Option<(LongWorkUnit, u32)> {
        let intent = &context.intent;
        (intent.child_study_sha256.is_some()
            && intent.long_attempt_seconds == Some(intent.max_attempt_seconds))
        .then(|| {
            intent
                .long_work_unit
                .clone()
                .map(|unit| (unit, intent.max_attempt_seconds))
        })
        .flatten()
    }

    /// Stop both halves; the attempt is settled only when each proves it.
    fn settle(&mut self) -> anyhow::Result<()> {
        let session = self.driver.abort();
        let process = self.inner.abort();
        session.and(process)
    }

    /// Stop the session driver once, then the game tree one bounded step per poll: a slow settlement never blocks the
    /// controller, and the outcome is reported only once the tree is proven gone.
    fn begin_settling(&mut self, outcome: Outcome) -> Progress {
        self.session = None;
        if self.driver.abort().is_err() {
            return Self::unverified();
        }
        self.settle_step(outcome)
    }

    fn settle_step(&mut self, outcome: Outcome) -> Progress {
        match self.inner.abort_step() {
            StopStep::Pending => {
                self.settling = Some(outcome);
                Progress::Running
            }
            StopStep::Unsettled => Self::unverified(),
            StopStep::Settled => match outcome {
                Outcome::Fail(class, code) => Progress::Failed { class, code },
                Outcome::Done {
                    record,
                    unit,
                    window,
                    index,
                } => {
                    // Only a proven settlement lets the host attest the record as settled before it is verified.
                    let mut record = record;
                    record.settled = true;
                    if let Err(code) = verify(&record, &unit, window) {
                        return Progress::Failed {
                            class: FailureClass::Integrity,
                            code: code.into(),
                        };
                    }
                    Progress::Finished {
                        evidence: AttemptEvidenceV1 {
                            thermal: None,
                            schema_version: 1,
                            slot_index: index,
                            // No FPS exists: the record is the evidence, this only has to be a valid placeholder.
                            average_fps: 1.0,
                            native_completion: None,
                            long_session: Some(record),
                        },
                    }
                }
            },
        }
    }

    fn unverified() -> Progress {
        Progress::Failed {
            class: FailureClass::OwnershipUnresolved,
            code: "SettlementUnverified".into(),
        }
    }
}

impl<H: AttemptHost, D: LongSessionDriver> AttemptHost for LongAttemptHost<H, D> {
    fn observe_desktop_idle(&mut self, idle_at: Option<std::time::Instant>) {
        self.inner.observe_desktop_idle(idle_at);
    }
    fn observe_user_input_idle(&mut self, input_idle_at: Option<std::time::Instant>) {
        self.inner.observe_user_input_idle(input_idle_at);
    }
    fn health_blocked(&mut self) -> bool {
        self.inner.health_blocked()
    }
    fn set_enabled_factors(&mut self, factors: &[ResearchFactor]) {
        self.inner.set_enabled_factors(factors);
    }
    fn set_treatments(&mut self, treatments: Option<Vec<Treatment>>) {
        self.inner.set_treatments(treatments);
    }
    fn attempt_index(&mut self) -> Option<u32> {
        self.inner.attempt_index()
    }
    fn attempt_treatment(&mut self) -> Option<Treatment> {
        self.inner.attempt_treatment()
    }
    fn check_prefixes(
        &mut self,
        arms: &[ProtonSource],
    ) -> Result<Option<super::prefix_identity::PrefixClass>, &'static str> {
        self.inner.check_prefixes(arms)
    }
    fn game_process(&mut self) -> Option<ChildTarget> {
        self.inner.game_process()
    }
    fn safe_window(&mut self) -> bool {
        self.inner.safe_window()
    }
    fn arm_spec(&mut self, slot: &AttemptSlot) -> anyhow::Result<ArmSpec> {
        self.inner.arm_spec(slot)
    }

    fn launch(&mut self, slot: &AttemptSlot, context: &DispatchContextV1) -> anyhow::Result<()> {
        let is_long_study = context.intent.long_attempt_seconds
            == Some(context.intent.max_attempt_seconds)
            && context.intent.child_study_sha256.is_some();
        let long = Self::long_of(context);
        if is_long_study && long.is_none() {
            return Err(LaunchFailure {
                class: FailureClass::Environment,
                code: "BlockedNoComparableLongWorkUnit",
            }
            .into());
        }
        self.inner.launch(slot, context)?;
        if let Some((unit, window)) = long {
            if let Err(code) = self.driver.start(&unit) {
                // The game was launched; stop it and prove it gone before reporting the refusal.
                self.settle()?;
                return Err(LaunchFailure {
                    class: FailureClass::Environment,
                    code,
                }
                .into());
            }
            self.session = Some((unit, window));
        }
        Ok(())
    }

    fn poll(&mut self) -> Progress {
        if let Some(outcome) = self.settling.take() {
            return self.settle_step(outcome);
        }
        let Some((unit, window)) = self.session.clone() else {
            return self.inner.poll();
        };
        // The inner host still watches the game (a GPU reset, an interrupted desktop, an exit); its
        // failures win. No native benchmark result is expected, so it never finishes a long attempt.
        if let Progress::Failed { class, code } = self.inner.poll() {
            self.session = None;
            let _ = self.driver.abort();
            return Progress::Failed { class, code };
        }
        let sighted = self.inner.game_process().map(|target| SessionProcess {
            pid: target.pid,
            start_ticks: target.start_ticks,
            uid: target.uid,
        });
        self.driver.observe_process(sighted);
        match self.driver.poll() {
            LongProgress::Running => Progress::Running,
            LongProgress::Failed(code) => {
                self.begin_settling(Outcome::Fail(FailureClass::Incompatible, code.into()))
            }
            LongProgress::Completed(record) => {
                let Some(index) = self.inner.attempt_index() else {
                    return self.begin_settling(Outcome::Fail(
                        FailureClass::Integrity,
                        "NoLaunchedAttempt".into(),
                    ));
                };
                // A driver cannot know the game is gone: the host stops both halves before it attests the record.
                self.begin_settling(Outcome::Done {
                    record,
                    unit,
                    window,
                    index,
                })
            }
        }
    }

    fn abort(&mut self) -> anyhow::Result<()> {
        self.session = None;
        let session = self.driver.abort();
        let process = self.inner.abort();
        session.and(process)
    }

    fn tool_file_sha256(&mut self, relative: &str) -> Option<String> {
        self.inner.tool_file_sha256(relative)
    }

    fn abort_step(&mut self) -> super::steam_host::StopStep {
        self.session = None;
        let session = self.driver.abort();
        let process = self.inner.abort_step();
        if session.is_err() {
            return super::steam_host::StopStep::Unsettled;
        }
        process
    }
}

#[cfg(test)]
#[path = "long_host_tests.rs"]
mod tests;
