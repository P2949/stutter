//! The real `AttemptHost` state logic over a narrow `SteamPort`. Only the port touches Steam, the
//! desktop or the game; this layer decides safe-window gating, one launch per attempt, and how an
//! exit without a fresh native result is classified. The production port is qualified separately
//! against a live Steam in a safe window.

use std::path::PathBuf;

use super::{
    AttemptSlot,
    dispatch_context::{ArmSpec, DispatchContextV1},
    driver::{AttemptHost, Progress},
    features,
    report::AttemptEvidenceV1,
};

#[derive(Clone, Debug, PartialEq)]
pub enum GameState {
    /// Steam accepted the launch but no game process is attributable yet.
    Starting,
    Running,
    /// A native result passed file verification against the supplied launch observations.
    /// The observation source must independently establish their authenticity.
    Result {
        completion: Box<super::cyberpunk::CyberpunkBenchmarkCompletion>,
    },
    InvalidNativeResult,
    /// The desktop became occupied or its idle observation expired.
    DesktopInterrupted,
    /// The GPU hung or was reset during the attempt: the result is void and the device is suspect.
    GpuReset,
    /// The game process tree ended without a fresh native result.
    ExitedWithoutResult,
    /// Something contradicted the launch identity (wrong game, wrong tool, second game).
    IdentityMismatch,
}

pub trait SteamPort {
    fn observe_desktop_idle(&mut self, idle_at: Option<std::time::Instant>);
    fn observe_user_input_idle(&mut self, input_idle_at: Option<std::time::Instant>);
    /// Objectively free of unrelated user or game work; never forced.
    fn safe_window(&mut self) -> bool;
    /// A durable health stop is on record.
    fn health_blocked(&mut self) -> bool {
        false
    }
    /// The sighted game process of the current attempt, once attested (`None` before).
    fn game_process(&mut self) -> Option<super::action_child::ChildTarget> {
        None
    }
    /// Resolve the exact installed tool directory for the arm, or fail closed.
    fn tool_dir(&mut self, slot: &AttemptSlot) -> anyhow::Result<PathBuf>;
    fn study_compat_data(&self) -> PathBuf;
    fn library_paths(&self) -> (PathBuf, PathBuf);
    fn launch(&mut self, slot: &AttemptSlot, context: &DispatchContextV1) -> anyhow::Result<()>;
    fn state(&mut self) -> GameState;
    /// Stop only the process tree this port launched. Must be idempotent.
    fn stop(&mut self) -> anyhow::Result<()>;
    /// One bounded step of settling the launched tree: `Settled` when the independent witness proves it gone,
    /// `Pending` while the port's settlement window is still open, `Unsettled` once it is not. The default is the
    /// blocking `stop`, for ports with nothing to wait for.
    fn stop_step(&mut self) -> StopStep {
        if self.stop().is_ok() {
            StopStep::Settled
        } else {
            StopStep::Unsettled
        }
    }
}

/// The result of one settlement step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopStep {
    Settled,
    Pending,
    Unsettled,
}

pub struct SteamAttemptHost<P: SteamPort> {
    port: P,
    current: Option<u32>,
    /// An attempt whose outcome is decided but whose launched tree is not yet proven gone.
    settling: Option<Progress>,
    /// The factors enabled for the attempts that follow; empty is the observe-only baseline.
    factors: Vec<super::ResearchFactor>,
    /// The running study's per-attempt treatments (none: the attempts are Proton arms).
    treatments: Option<Vec<super::Treatment>>,
    /// The arm's frozen installation, to bind the mapped runtime to when the result arrives.
    tool_dir: Option<PathBuf>,
    /// The running attempt only initializes its arm's prefix (`DispatchContextV1.intent.initialization`).
    initializing: bool,
    current_arm: Option<super::ProtonSource>,
    warm_since: Option<std::time::Instant>,
    init_settle: std::time::Duration,
    /// Where the kernel's hwmon files are read (replaceable for tests).
    hwmon_root: PathBuf,
    /// The temperatures when the running attempt launched.
    thermal_start: Option<(
        super::thermal::ThermalReading,
        super::thermal::ThermalReading,
    )>,
}

#[path = "steam_host_init.rs"]
mod init;

impl<P: SteamPort> SteamAttemptHost<P> {
    /// The study's factors and whether this attempt runs them On. A within-source Off attempt runs
    /// the catalog's explicit control environment for the same factors, never an empty one: for
    /// NTSync the control is `PROTON_NO_NTSYNC=1`, and an empty environment would give both arms
    /// the same one.
    fn factors_for(&self, slot: &AttemptSlot) -> (Vec<super::ResearchFactor>, bool) {
        let off = self
            .treatments
            .as_ref()
            .and_then(|treatments| treatments.get(slot.index as usize))
            .is_some_and(|treatment| *treatment == super::Treatment::Off);
        (self.factors.clone(), !off)
    }

    /// One settlement step for an attempt whose outcome is already decided. `Settled` delivers the outcome;
    /// `Pending` keeps it and the attempt running (ownership is retained, nothing is reported); a settlement that
    /// cannot be proven within the port's window fails the attempt as unresolved ownership.
    pub(super) fn after_settlement(&mut self, outcome: Progress) -> Progress {
        match self.port.stop_step() {
            StopStep::Settled => {
                self.current = None;
                outcome
            }
            StopStep::Pending => {
                self.settling = Some(outcome);
                Progress::Running
            }
            StopStep::Unsettled => Progress::Failed {
                class: crate::autotune::research::driver::FailureClass::OwnershipUnresolved,
                code: "SettlementUnverified".into(),
            },
        }
    }

    pub fn new(port: P) -> Self {
        Self {
            port,
            current: None,
            settling: None,
            factors: Vec::new(),
            treatments: None,
            tool_dir: None,
            initializing: false,
            current_arm: None,
            warm_since: None,
            init_settle: init::INIT_SETTLE,
            hwmon_root: PathBuf::from("/sys/class/hwmon"),
            thermal_start: None,
        }
    }
}

impl<P: SteamPort> AttemptHost for SteamAttemptHost<P> {
    fn observe_desktop_idle(&mut self, idle_at: Option<std::time::Instant>) {
        self.port.observe_desktop_idle(idle_at);
    }

    fn observe_user_input_idle(&mut self, input_idle_at: Option<std::time::Instant>) {
        self.port.observe_user_input_idle(input_idle_at);
    }

    fn health_blocked(&mut self) -> bool {
        self.port.health_blocked()
    }

    fn safe_window(&mut self) -> bool {
        self.current.is_none() && self.port.safe_window()
    }

    fn set_enabled_factors(&mut self, factors: &[super::ResearchFactor]) {
        self.factors = factors.to_vec();
    }

    fn check_prefixes(
        &mut self,
        arms: &[super::ProtonSource],
    ) -> Result<Option<super::prefix_identity::PrefixClass>, &'static str> {
        let mut inputs = Vec::new();
        for arm in arms {
            let slot = AttemptSlot {
                study_sha256: String::new(),
                context_sha256: String::new(),
                index: 0,
                arm: *arm,
            };
            let tool = self
                .port
                .tool_dir(&slot)
                .map_err(|_| "PrefixToolUnavailable")?;
            inputs.push((self.port.study_compat_data().join(arm.dir_name()), tool));
        }
        super::prefix_identity::check(&inputs).map(Some)
    }

    fn set_treatments(&mut self, treatments: Option<Vec<super::Treatment>>) {
        self.treatments = treatments;
    }

    fn attempt_index(&mut self) -> Option<u32> {
        self.current
    }

    fn attempt_treatment(&mut self) -> Option<super::Treatment> {
        let index = usize::try_from(self.current?).ok()?;
        self.treatments.as_ref()?.get(index).copied()
    }

    fn game_process(&mut self) -> Option<super::action_child::ChildTarget> {
        self.port.game_process()
    }

    fn arm_spec(&mut self, slot: &AttemptSlot) -> anyhow::Result<ArmSpec> {
        use super::driver_failure::{FailureClass, LaunchFailure};
        // (a within-source study applies its factor only to the attempts treated On)
        let (steamapps, proton_manifest) = self.port.library_paths();
        Ok(ArmSpec {
            tool_dir: self.port.tool_dir(slot).map_err(|_| LaunchFailure {
                class: FailureClass::Integrity,
                code: "ScheduledToolUnavailable",
            })?,
            // Every arm runs in its own private prefix below the study compatdata root.
            study_compat_data: self.port.study_compat_data().join(slot.arm.dir_name()),
            // Only the factors this study enabled (none for screening): a blocked or conflicting
            // factor refuses the arm rather than running it at baseline unannounced.
            arm_env: {
                let (factors, on) = self.factors_for(slot);
                features::arm_env(slot.arm, &factors, on)
                    .map_err(|(f, b)| anyhow::anyhow!("feature {f:?} blocked: {b:?}"))?
            },
            steamapps,
            proton_manifest,
        })
    }

    fn launch(&mut self, slot: &AttemptSlot, context: &DispatchContextV1) -> anyhow::Result<()> {
        use super::driver_failure::{FailureClass, LaunchFailure};
        LaunchFailure::require(
            self.current.is_none(),
            FailureClass::OwnershipUnresolved,
            "SettlementUnproven",
        )?;
        let tool_dir = self.port.tool_dir(slot).map_err(|_| LaunchFailure {
            class: FailureClass::Integrity,
            code: "ScheduledToolUnavailable",
        })?;
        self.current = Some(slot.index);
        self.current_arm = Some(slot.arm);
        self.initializing = context.intent.initialization;
        self.warm_since = None;
        self.tool_dir = Some(tool_dir);
        self.thermal_start = Some((
            super::thermal::read(&self.hwmon_root),
            super::thermal::read_limits(&self.hwmon_root),
        ));
        self.port.launch(slot, context)
    }

    fn poll(&mut self) -> Progress {
        let Some(index) = self.current else {
            return Progress::Failed {
                class: crate::autotune::research::driver::FailureClass::Integrity,
                code: "NoLaunchedAttempt".into(),
            };
        };
        if let Some(outcome) = self.settling.take() {
            return self.after_settlement(outcome);
        }
        let state = self.port.state();
        let state = if self.initializing {
            match self.init_poll(index, state) {
                Ok(progress) => return progress,
                Err(other) => other,
            }
        } else {
            state
        };
        let outcome = match state {
            GameState::Starting | GameState::Running => return Progress::Running,
            GameState::Result { mut completion } => {
                // What the game had mapped is compared with the arm's own installation; a runtime
                // that was never observed stays unknown rather than matching.
                completion.runtime_binding = completion.loaded_runtime.as_ref().map(|runtime| {
                    self.tool_dir
                        .as_deref()
                        .map_or(super::loaded_runtime::RuntimeBinding::Unknown, |dir| {
                            runtime.bind_to(dir)
                        })
                });
                if completion.runtime_binding
                    == Some(super::loaded_runtime::RuntimeBinding::Mismatched)
                {
                    Progress::Failed {
                        class: super::driver::FailureClass::Integrity,
                        code: "RuntimeIdentityMismatch".into(),
                    }
                } else {
                    Progress::Finished {
                        evidence: AttemptEvidenceV1 {
                            thermal: self.thermal_start.take().map(|(start, limit)| {
                                super::thermal::ThermalContext {
                                    start,
                                    limit,
                                    end: super::thermal::read(&self.hwmon_root),
                                }
                            }),
                            long_session: None,
                            schema_version: 1,
                            slot_index: index,
                            average_fps: completion.metrics.average_fps,
                            native_completion: Some(completion),
                        },
                    }
                }
            }
            GameState::InvalidNativeResult => {
                return Progress::Failed {
                    class: crate::autotune::research::driver::FailureClass::Integrity,
                    code: "InvalidNativeResult".into(),
                };
            }
            GameState::DesktopInterrupted => {
                return Progress::Failed {
                    class: crate::autotune::research::driver::FailureClass::Environment,
                    code: "DesktopInterrupted".into(),
                };
            }
            GameState::GpuReset => {
                return Progress::Failed {
                    class: crate::autotune::research::driver::FailureClass::Health,
                    code: "GpuReset".into(),
                };
            }
            GameState::ExitedWithoutResult => Progress::Failed {
                class: crate::autotune::research::driver::FailureClass::Incompatible,
                code: "ExitedWithoutNativeResult".into(),
            },
            GameState::IdentityMismatch => Progress::Failed {
                class: crate::autotune::research::driver::FailureClass::Integrity,
                code: "IdentityMismatch".into(),
            },
        };
        // The outcome is known, but nothing is reported until the launched tree is proven gone: the witness is
        // asked one step per poll, so a slow settlement never blocks the controller.
        self.after_settlement(outcome)
    }

    fn abort(&mut self) -> anyhow::Result<()> {
        self.port.stop()?;
        self.current = None;
        Ok(())
    }

    fn tool_file_sha256(&mut self, relative: &str) -> Option<String> {
        // A tool-relative path only: nothing that climbs out of the tool directory is read.
        if relative.starts_with('/') || relative.split('/').any(|part| part == "..") {
            return None;
        }
        let path = self.tool_dir.as_ref()?.join(relative);
        let meta = std::fs::metadata(&path).ok()?;
        (meta.is_file() && meta.len() <= 64 * 1024 * 1024)
            .then(|| std::fs::read(&path).ok())
            .flatten()
            .map(|bytes| stutter_core::hash::sha256_hex(&bytes))
    }

    fn abort_step(&mut self) -> StopStep {
        let step = self.port.stop_step();
        if step == StopStep::Settled {
            self.current = None;
        }
        step
    }
}

#[cfg(test)]
#[path = "steam_host_tests.rs"]
mod tests;
