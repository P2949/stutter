use std::sync::{Arc, Mutex};

use super::*;
use crate::autotune::research::{dispatch_context::ArmSpec, driver_failure::LaunchFailure};

#[derive(Default)]
struct Script {
    states: Vec<GameState>,
    launched: u32,
    stopped: u32,
    health_blocked: bool,
    unsafe_window: bool,
    stop_fails: bool,
    refuse_with: Option<&'static str>,
}

struct Port(Arc<Mutex<Script>>);

impl SteamPort for Port {
    fn observe_desktop_idle(&mut self, _: Option<std::time::Instant>) {}
    fn observe_user_input_idle(&mut self, _: Option<std::time::Instant>) {}
    fn safe_window(&mut self) -> bool {
        !self.0.lock().unwrap().unsafe_window
    }
    fn health_blocked(&mut self) -> bool {
        self.0.lock().unwrap().health_blocked
    }
    fn tool_dir(&mut self, _: &AttemptSlot) -> anyhow::Result<std::path::PathBuf> {
        Ok("/t".into())
    }
    fn study_compat_data(&self) -> std::path::PathBuf {
        "/s".into()
    }
    fn library_paths(&self) -> (std::path::PathBuf, std::path::PathBuf) {
        ("/a".into(), "/m".into())
    }
    fn launch(&mut self, _: &AttemptSlot, _: &DispatchContextV1) -> anyhow::Result<()> {
        let mut script = self.0.lock().unwrap();
        if let Some(code) = script.refuse_with {
            return Err(LaunchFailure {
                class: crate::autotune::research::driver_failure::FailureClass::Environment,
                code,
            }
            .into());
        }
        script.launched += 1;
        Ok(())
    }
    fn state(&mut self) -> GameState {
        let mut script = self.0.lock().unwrap();
        if script.states.len() > 1 {
            script.states.remove(0)
        } else {
            script.states.first().cloned().unwrap_or(GameState::Running)
        }
    }
    fn stop(&mut self) -> anyhow::Result<()> {
        let mut script = self.0.lock().unwrap();
        script.stopped += 1;
        anyhow::ensure!(!script.stop_fails, "unsettled");
        Ok(())
    }
}

fn host(script: Script) -> (SteamInitHost<Port>, Arc<Mutex<Script>>) {
    let shared = Arc::new(Mutex::new(script));
    let host = SteamInitHost::new(
        Port(shared.clone()),
        Box::new(|arm| {
            let mut intent = crate::autotune::research::tests::intent();
            intent.uid = crate::syscall::geteuid();
            let slot = AttemptSlot {
                study_sha256: "e".repeat(64),
                context_sha256: "f".repeat(64),
                index: 0,
                arm,
            };
            Ok((
                slot.clone(),
                DispatchContextV1 {
                    schema_version: 1,
                    intent,
                    slot,
                    tickets: Default::default(),
                    spec: ArmSpec {
                        tool_dir: "/t".into(),
                        study_compat_data: "/s".into(),
                        arm_env: Default::default(),
                        steamapps: "/a".into(),
                        proton_manifest: "/m".into(),
                    },
                },
            ))
        }),
    );
    (host, shared)
}

#[test]
fn the_launch_passes_the_same_admission_and_the_game_ending_is_settled_by_the_port() {
    let (mut host, script) = host(Script {
        states: vec![
            GameState::Starting,
            GameState::Running,
            GameState::ExitedWithoutResult,
        ],
        ..Script::default()
    });
    host.launch(ProtonSource::Ge).unwrap();
    assert_eq!(host.process(), Ok(InitProcess::Running));
    assert_eq!(host.process(), Ok(InitProcess::Running));
    assert_eq!(host.process(), Ok(InitProcess::Settled));
    let seen = script.lock().unwrap();
    assert_eq!((seen.launched, seen.stopped), (1, 1));
}

#[test]
fn a_blocked_gpu_a_closed_window_or_a_typed_refusal_launches_nothing() {
    let (mut sick, script) = host(Script {
        health_blocked: true,
        ..Script::default()
    });
    assert_eq!(sick.launch(ProtonSource::Ge), Err("BlockedGpuStability"));
    let (mut busy, _) = host(Script {
        unsafe_window: true,
        ..Script::default()
    });
    assert_eq!(busy.launch(ProtonSource::Ge), Err("SafeWindowClosed"));
    let (mut refused, _) = host(Script {
        refuse_with: Some("BlockedDiskReserve"),
        ..Script::default()
    });
    assert_eq!(refused.launch(ProtonSource::Ge), Err("BlockedDiskReserve"));
    assert_eq!(script.lock().unwrap().launched, 0);
}

#[test]
fn a_voided_launch_is_settled_before_it_is_reported_and_unproven_settlement_is_never_hidden() {
    let (mut host, script) = host(Script {
        states: vec![GameState::GpuReset],
        ..Script::default()
    });
    host.launch(ProtonSource::Ge).unwrap();
    assert_eq!(host.process(), Err("InitLaunchVoided"));
    assert_eq!(script.lock().unwrap().stopped, 1);

    let (mut stuck, _) = host_with_stop_failure();
    stuck.launch(ProtonSource::Ge).unwrap();
    assert_eq!(stuck.process(), Err("SettlementUnproven"));
    assert_eq!(stuck.terminate(), Err("SettlementUnproven"));
}

fn host_with_stop_failure() -> (SteamInitHost<Port>, Arc<Mutex<Script>>) {
    host(Script {
        states: vec![GameState::ExitedWithoutResult],
        stop_fails: true,
        ..Script::default()
    })
}
