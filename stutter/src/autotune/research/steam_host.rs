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
    /// A fresh native result was verified for exactly this launch.
    Result {
        average_fps: f64,
    },
    /// The game process tree ended without a fresh native result.
    ExitedWithoutResult,
    /// Something contradicted the launch identity (wrong game, wrong tool, second game).
    IdentityMismatch,
}

pub trait SteamPort {
    /// Objectively free of unrelated user or game work; never forced.
    fn safe_window(&mut self) -> bool;
    /// Resolve the exact installed tool directory for the arm, or fail closed.
    fn tool_dir(&mut self, slot: &AttemptSlot) -> anyhow::Result<PathBuf>;
    fn study_compat_data(&self) -> PathBuf;
    fn library_paths(&self) -> (PathBuf, PathBuf);
    fn launch(&mut self, slot: &AttemptSlot) -> anyhow::Result<()>;
    fn state(&mut self) -> GameState;
    /// Stop only the process tree this port launched. Must be idempotent.
    fn stop(&mut self);
}

pub struct SteamAttemptHost<P: SteamPort> {
    port: P,
    current: Option<u32>,
}

impl<P: SteamPort> SteamAttemptHost<P> {
    pub fn new(port: P) -> Self {
        Self {
            port,
            current: None,
        }
    }
}

impl<P: SteamPort> AttemptHost for SteamAttemptHost<P> {
    fn safe_window(&mut self) -> bool {
        self.current.is_none() && self.port.safe_window()
    }

    fn arm_spec(&mut self, slot: &AttemptSlot) -> anyhow::Result<ArmSpec> {
        let (steamapps, proton_manifest) = self.port.library_paths();
        Ok(ArmSpec {
            tool_dir: self.port.tool_dir(slot)?,
            study_compat_data: self.port.study_compat_data(),
            // Observe-only screening: no factor is enabled, so the baseline is the empty set.
            arm_env: features::arm_env(slot.arm, &[])
                .map_err(|(f, b)| anyhow::anyhow!("feature {f:?} blocked: {b:?}"))?,
            steamapps,
            proton_manifest,
        })
    }

    fn launch(&mut self, slot: &AttemptSlot, _: &DispatchContextV1) -> anyhow::Result<()> {
        anyhow::ensure!(self.current.is_none(), "an attempt is already launched");
        self.current = Some(slot.index);
        self.port.launch(slot)
    }

    fn poll(&mut self) -> Progress {
        let Some(index) = self.current else {
            return Progress::Failed {
                code: "NoLaunchedAttempt".into(),
            };
        };
        match self.port.state() {
            GameState::Starting | GameState::Running => Progress::Running,
            GameState::Result { average_fps } => {
                self.port.stop();
                self.current = None;
                Progress::Finished {
                    evidence: AttemptEvidenceV1 {
                        schema_version: 1,
                        slot_index: index,
                        average_fps,
                    },
                }
            }
            GameState::ExitedWithoutResult => {
                self.current = None;
                Progress::Failed {
                    code: "ExitedWithoutNativeResult".into(),
                }
            }
            GameState::IdentityMismatch => {
                self.port.stop();
                self.current = None;
                Progress::Failed {
                    code: "IdentityMismatch".into(),
                }
            }
        }
    }

    fn abort(&mut self) {
        self.port.stop();
        self.current = None;
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
        fn launch(&mut self, _: &AttemptSlot) -> anyhow::Result<()> {
            self.launches += 1;
            Ok(())
        }
        fn state(&mut self) -> GameState {
            self.states.remove(0)
        }
        fn stop(&mut self) {
            self.stops += 1;
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

    #[test]
    fn a_fresh_native_result_finishes_once_and_stops_the_owned_tree() {
        let mut host = host(vec![
            GameState::Starting,
            GameState::Running,
            GameState::Result { average_fps: 71.5 },
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
}
