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
}

pub struct SteamAttemptHost<P: SteamPort> {
    port: P,
    current: Option<u32>,
    /// The factors enabled for the attempts that follow; empty is the observe-only baseline.
    factors: Vec<super::ResearchFactor>,
    /// The running study's per-attempt treatments (none: the attempts are Proton arms).
    treatments: Option<Vec<super::Treatment>>,
    /// The arm's frozen installation, to bind the mapped runtime to when the result arrives.
    tool_dir: Option<PathBuf>,
}

impl<P: SteamPort> SteamAttemptHost<P> {
    /// The factors this attempt runs with: the study's factors, except that in a within-source study
    /// the Off attempts run at baseline.
    fn factors_for(&self, slot: &AttemptSlot) -> Vec<super::ResearchFactor> {
        let off = self
            .treatments
            .as_ref()
            .and_then(|treatments| treatments.get(slot.index as usize))
            .is_some_and(|treatment| *treatment == super::Treatment::Off);
        if off {
            Vec::new()
        } else {
            self.factors.clone()
        }
    }

    pub fn new(port: P) -> Self {
        Self {
            port,
            current: None,
            factors: Vec::new(),
            treatments: None,
            tool_dir: None,
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

    fn set_treatments(&mut self, treatments: Option<Vec<super::Treatment>>) {
        self.treatments = treatments;
    }

    fn attempt_treatment(&mut self) -> Option<super::Treatment> {
        let index = usize::try_from(self.current?).ok()?;
        self.treatments.as_ref()?.get(index).copied()
    }

    fn game_process(&mut self) -> Option<super::action_child::ChildTarget> {
        self.port.game_process()
    }

    fn arm_spec(&mut self, slot: &AttemptSlot) -> anyhow::Result<ArmSpec> {
        // (a within-source study applies its factor only to the attempts treated On)
        let (steamapps, proton_manifest) = self.port.library_paths();
        Ok(ArmSpec {
            tool_dir: self.port.tool_dir(slot)?,
            // Every arm runs in its own private prefix below the study compatdata root.
            study_compat_data: self.port.study_compat_data().join(slot.arm.dir_name()),
            // Only the factors this study enabled (none for screening): a blocked or conflicting
            // factor refuses the arm rather than running it at baseline unannounced.
            arm_env: features::arm_env(slot.arm, &self.factors_for(slot))
                .map_err(|(f, b)| anyhow::anyhow!("feature {f:?} blocked: {b:?}"))?,
            steamapps,
            proton_manifest,
        })
    }

    fn launch(&mut self, slot: &AttemptSlot, context: &DispatchContextV1) -> anyhow::Result<()> {
        anyhow::ensure!(self.current.is_none(), "an attempt is already launched");
        self.current = Some(slot.index);
        self.tool_dir = self.port.tool_dir(slot).ok();
        self.port.launch(slot, context)
    }

    fn poll(&mut self) -> Progress {
        let Some(index) = self.current else {
            return Progress::Failed {
                code: "NoLaunchedAttempt".into(),
            };
        };
        match self.port.state() {
            GameState::Starting | GameState::Running => Progress::Running,
            GameState::Result { mut completion } => {
                if self.port.stop().is_err() {
                    return Progress::Failed {
                        code: "SettlementUnverified".into(),
                    };
                }
                self.current = None;
                // What the game had mapped is compared with the arm's own installation; a runtime
                // that was never observed stays unknown rather than matching.
                completion.runtime_binding = completion.loaded_runtime.as_ref().map(|runtime| {
                    self.tool_dir
                        .as_deref()
                        .map_or(super::loaded_runtime::RuntimeBinding::Unknown, |dir| {
                            runtime.bind_to(dir)
                        })
                });
                Progress::Finished {
                    evidence: AttemptEvidenceV1 {
                        schema_version: 1,
                        slot_index: index,
                        average_fps: completion.metrics.average_fps,
                        native_completion: Some(completion),
                    },
                }
            }
            GameState::InvalidNativeResult => Progress::Failed {
                code: "InvalidNativeResult".into(),
            },
            GameState::DesktopInterrupted => Progress::Failed {
                code: "DesktopInterrupted".into(),
            },
            GameState::GpuReset => Progress::Failed {
                code: "GpuReset".into(),
            },
            GameState::ExitedWithoutResult => {
                if self.port.stop().is_err() {
                    return Progress::Failed {
                        code: "SettlementUnverified".into(),
                    };
                }
                self.current = None;
                Progress::Failed {
                    code: "ExitedWithoutNativeResult".into(),
                }
            }
            GameState::IdentityMismatch => {
                if self.port.stop().is_err() {
                    return Progress::Failed {
                        code: "SettlementUnverified".into(),
                    };
                }
                self.current = None;
                Progress::Failed {
                    code: "IdentityMismatch".into(),
                }
            }
        }
    }

    fn abort(&mut self) -> anyhow::Result<()> {
        self.port.stop()?;
        self.current = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autotune::research::ProtonSource;

    struct Port {
        safe: bool,
        states: Vec<GameState>,
        launches: u32,
        stops: u32,
    }

    impl SteamPort for Port {
        fn observe_desktop_idle(&mut self, _: Option<std::time::Instant>) {}
        fn observe_user_input_idle(&mut self, _: Option<std::time::Instant>) {}
        fn safe_window(&mut self) -> bool {
            self.safe
        }
        fn tool_dir(&mut self, _: &AttemptSlot) -> anyhow::Result<PathBuf> {
            Ok("/tools/GE".into())
        }
        fn study_compat_data(&self) -> PathBuf {
            "/study/compatdata".into()
        }
        fn library_paths(&self) -> (PathBuf, PathBuf) {
            ("/steam/steamapps".into(), "/manifest.json".into())
        }
        fn launch(&mut self, _: &AttemptSlot, _: &DispatchContextV1) -> anyhow::Result<()> {
            self.launches += 1;
            Ok(())
        }
        fn state(&mut self) -> GameState {
            self.states.remove(0)
        }
        fn stop(&mut self) -> anyhow::Result<()> {
            self.stops += 1;
            Ok(())
        }
    }

    fn slot() -> AttemptSlot {
        AttemptSlot {
            study_sha256: "e".repeat(64),
            context_sha256: "f".repeat(64),
            index: 3,
            arm: ProtonSource::Ge,
        }
    }

    fn host(states: Vec<GameState>) -> SteamAttemptHost<Port> {
        SteamAttemptHost::new(Port {
            safe: true,
            states,
            launches: 0,
            stops: 0,
        })
    }

    #[test]
    fn each_arm_is_armed_with_its_own_private_compat_directory() {
        let mut host = host(vec![]);
        let ge = host.arm_spec(&slot()).unwrap();
        let mut other = slot();
        other.arm = ProtonSource::CachyOs;
        let cachy = host.arm_spec(&other).unwrap();
        assert_eq!(
            ge.study_compat_data,
            std::path::Path::new("/study/compatdata/ge")
        );
        assert_eq!(
            cachy.study_compat_data,
            std::path::Path::new("/study/compatdata/cachy_os")
        );
    }

    fn launch(host: &mut SteamAttemptHost<Port>) {
        let spec = host.arm_spec(&slot()).unwrap();
        assert!(spec.arm_env.is_empty());
        let context = DispatchContextV1 {
            schema_version: 1,
            intent: crate::autotune::research::tests::intent(),
            slot: slot(),
            tickets: Default::default(),
            spec,
        };
        host.launch(&slot(), &context).unwrap();
    }

    fn completion() -> super::super::cyberpunk::CyberpunkBenchmarkCompletion {
        use super::super::cyberpunk::{
            CyberpunkBenchmarkCompletion, CyberpunkProcessIdentity, CyberpunkScenario,
            NativeBenchmarkMetrics,
        };
        CyberpunkBenchmarkCompletion {
            schema_version: 1,
            app_id: 1_091_500,
            build_id: 42,
            scene: CyberpunkScenario::BuiltinBenchmark,
            settings_sha256: "c".repeat(64),
            proton_source: ProtonSource::Ge,
            proton_identity_sha256: "d".repeat(64),
            process: CyberpunkProcessIdentity {
                pid: stutter_core::ids::Pid::new(1234),
                start_ticks: 1,
                uid: crate::syscall::geteuid(),
                app_id: 1_091_500,
                build_id: 42,
                executable_sha256: "b".repeat(64),
            },
            started_unix_nanos: 1,
            completed_unix_nanos: 2,
            result_directory: "result".into(),
            metrics: NativeBenchmarkMetrics {
                average_fps: 71.5,
                minimum_fps: None,
                maximum_fps: None,
            },
            context: None,
            loaded_runtime: None,
            runtime_binding: None,
            files: vec![],
        }
    }

    #[test]
    fn a_fresh_native_result_finishes_once_and_stops_the_owned_tree() {
        let mut host = host(vec![
            GameState::Starting,
            GameState::Running,
            GameState::Result {
                completion: Box::new(completion()),
            },
        ]);
        assert!(host.safe_window());
        launch(&mut host);
        assert!(
            !host.safe_window(),
            "never a second launch while one is live"
        );
        assert_eq!(host.poll(), Progress::Running);
        assert_eq!(host.poll(), Progress::Running);
        let Progress::Finished { evidence } = host.poll() else {
            panic!("expected a result")
        };
        assert_eq!((evidence.slot_index, evidence.average_fps), (3, 71.5));
        assert_eq!(host.port.stops, 1);
        assert_eq!(host.port.launches, 1);
    }

    #[test]
    fn exit_without_a_native_result_and_identity_mismatch_fail_closed() {
        let mut host = host(vec![GameState::ExitedWithoutResult]);
        launch(&mut host);
        assert_eq!(
            host.poll(),
            Progress::Failed {
                code: "ExitedWithoutNativeResult".into()
            }
        );
        let mut host = host_with_mismatch();
        launch(&mut host);
        assert_eq!(
            host.poll(),
            Progress::Failed {
                code: "IdentityMismatch".into()
            }
        );
        assert_eq!(host.port.stops, 1);
    }

    #[test]
    fn desktop_interruption_requests_settlement_before_any_seal() {
        let mut host = host(vec![GameState::DesktopInterrupted]);
        launch(&mut host);
        assert_eq!(
            host.poll(),
            Progress::Failed {
                code: "DesktopInterrupted".into()
            }
        );
        // The driver owns the abort/settlement step after a failed poll.
        assert_eq!(host.port.stops, 0);
        host.abort().unwrap();
        assert_eq!(host.port.stops, 1);
    }

    fn host_with_mismatch() -> SteamAttemptHost<Port> {
        host(vec![GameState::IdentityMismatch])
    }

    #[test]
    fn an_unsafe_window_blocks_and_a_poll_without_launch_is_a_failure() {
        let mut host = host(vec![]);
        host.port.safe = false;
        assert!(!host.safe_window());
        assert_eq!(
            host.poll(),
            Progress::Failed {
                code: "NoLaunchedAttempt".into()
            }
        );
    }

    #[test]
    fn a_result_carries_the_binding_of_the_mapped_runtime_to_the_arms_own_installation() {
        use super::super::loaded_runtime::{LoadedRuntime, RuntimeBinding};
        let runtime = |root: &str| LoadedRuntime {
            mapped_file_count: 3,
            wine_loaded: true,
            vkd3d_loaded: true,
            dxvk_loaded: true,
            proton_roots: vec![root.to_owned()],
            runtime_paths_sha256: "a".repeat(64),
        };
        let binding_for = |receipt: Option<LoadedRuntime>| {
            let mut result = completion();
            result.loaded_runtime = receipt;
            let mut host = host(vec![GameState::Result {
                completion: Box::new(result),
            }]);
            launch(&mut host);
            let Progress::Finished { evidence } = host.poll() else {
                panic!("a verified result finishes the attempt")
            };
            evidence.native_completion.unwrap().runtime_binding
        };
        // The arm's installation is /tools/GE.
        assert_eq!(
            binding_for(Some(runtime("/tools/GE/files"))),
            Some(RuntimeBinding::Matched)
        );
        assert_eq!(
            binding_for(Some(runtime("/run/host/tools/GE/files"))),
            Some(RuntimeBinding::Matched)
        );
        assert_eq!(
            binding_for(Some(runtime("/tools/CachyOS/files"))),
            Some(RuntimeBinding::Mismatched)
        );
        // Nothing captured proves nothing and records no binding.
        assert_eq!(binding_for(None), None);
    }

    #[test]
    fn an_arm_runs_at_baseline_unless_the_study_enabled_a_factor_and_a_blocked_factor_refuses_it() {
        use crate::autotune::research::ResearchFactor;
        let mut host = host(vec![]);
        assert!(host.arm_spec(&slot()).unwrap().arm_env.is_empty());
        // WineWayland on is a real environment for the Em build; it is not baseline.
        let mut em = slot();
        em.arm = ProtonSource::Em;
        host.set_enabled_factors(&[ResearchFactor::WineWayland]);
        assert_eq!(
            host.arm_spec(&em).unwrap().arm_env["PROTON_ENABLE_WAYLAND"],
            "1"
        );
        // A factor this build cannot activate refuses the arm instead of running it at baseline.
        host.set_enabled_factors(&[ResearchFactor::AntiLag2]);
        assert!(host.arm_spec(&slot()).is_err());
        // Clearing the factors returns to baseline.
        host.set_enabled_factors(&[]);
        assert!(host.arm_spec(&slot()).unwrap().arm_env.is_empty());
    }

    #[test]
    fn a_within_source_study_applies_its_factor_only_to_the_attempts_treated_on() {
        use crate::autotune::research::{ResearchFactor, Treatment};
        let mut host = host(vec![]);
        let mut em = slot();
        em.arm = ProtonSource::Em;
        host.set_enabled_factors(&[ResearchFactor::WineWayland]);
        host.set_treatments(Some(vec![Treatment::Off, Treatment::On, Treatment::On]));
        let at = |index: u32| AttemptSlot {
            index,
            ..em.clone()
        };
        // Off runs at baseline; On carries the factor; the study's own arm is the same build.
        assert!(host.arm_spec(&at(0)).unwrap().arm_env.is_empty());
        assert_eq!(
            host.arm_spec(&at(1)).unwrap().arm_env["PROTON_ENABLE_WAYLAND"],
            "1"
        );
        // The treatment of the running attempt follows the launched slot.
        host.current = Some(0);
        assert_eq!(host.attempt_treatment(), Some(Treatment::Off));
        host.current = Some(2);
        assert_eq!(host.attempt_treatment(), Some(Treatment::On));
        host.current = None;
        assert_eq!(host.attempt_treatment(), None);
        // A study without treatments applies its factor to every attempt, as before.
        host.set_treatments(None);
        assert_eq!(
            host.arm_spec(&at(0)).unwrap().arm_env["PROTON_ENABLE_WAYLAND"],
            "1"
        );
    }
}
