use std::{path::PathBuf, time::Duration};

use super::*;
use crate::autotune::research::{
    AttemptSlot, ProtonSource, dispatch_context::DispatchContextV1, driver::AttemptHost,
};

struct Port {
    root: PathBuf,
    state: GameState,
    stops: u32,
    stop_fails: bool,
}

impl SteamPort for Port {
    fn observe_desktop_idle(&mut self, _: Option<Instant>) {}
    fn observe_user_input_idle(&mut self, _: Option<Instant>) {}
    fn safe_window(&mut self) -> bool {
        true
    }
    fn tool_dir(&mut self, _: &AttemptSlot) -> anyhow::Result<PathBuf> {
        Ok("/tools/GE".into())
    }
    fn study_compat_data(&self) -> PathBuf {
        self.root.clone()
    }
    fn library_paths(&self) -> (PathBuf, PathBuf) {
        ("/a".into(), "/m".into())
    }
    fn launch(&mut self, _: &AttemptSlot, _: &DispatchContextV1) -> anyhow::Result<()> {
        Ok(())
    }
    fn state(&mut self) -> GameState {
        self.state.clone()
    }
    fn stop(&mut self) -> anyhow::Result<()> {
        self.stops += 1;
        anyhow::ensure!(!self.stop_fails, "unsettled");
        Ok(())
    }
}

fn slot() -> AttemptSlot {
    AttemptSlot {
        study_sha256: "e".repeat(64),
        context_sha256: "f".repeat(64),
        index: 0,
        arm: ProtonSource::Ge,
    }
}

fn started(root: &std::path::Path, state: GameState) -> SteamAttemptHost<Port> {
    let mut host = SteamAttemptHost::new(Port {
        root: root.to_path_buf(),
        state,
        stops: 0,
        stop_fails: false,
    });
    host.init_settle = Duration::ZERO;
    let mut intent = crate::autotune::research::tests::intent();
    intent.initialization = true;
    let spec = host.arm_spec(&slot()).unwrap();
    let context = DispatchContextV1 {
        schema_version: 1,
        intent,
        slot: slot(),
        tickets: Default::default(),
        spec,
    };
    host.launch(&slot(), &context).unwrap();
    host
}

fn create_prefix(root: &std::path::Path, with_settings: bool) {
    let compat = root.join("ge");
    let pfx = compat.join("pfx");
    std::fs::create_dir_all(&pfx).unwrap();
    for name in ["system.reg", "user.reg", "userdef.reg"] {
        std::fs::write(pfx.join(name), b"x").unwrap();
    }
    if with_settings {
        let settings = compat.join(SETTINGS_RELATIVE);
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(settings, b"{}").unwrap();
    }
}

#[test]
fn a_running_game_is_stopped_only_once_its_prefix_and_settings_exist_and_nothing_is_measured() {
    let root = tempfile::tempdir().unwrap();
    let mut host = started(root.path(), GameState::Running);
    assert!(
        matches!(host.poll(), Progress::Running),
        "cold: keep waiting"
    );
    create_prefix(root.path(), false);
    assert!(
        matches!(host.poll(), Progress::Running),
        "no settings file yet"
    );
    create_prefix(root.path(), true);
    let Progress::Finished { evidence } = host.poll() else {
        panic!("a ready prefix finishes the attempt");
    };
    assert!(evidence.native_completion.is_none());
    assert_eq!(evidence.average_fps, 1.0);
    assert_eq!(
        host.port.stops, 1,
        "the process tree is settled through the port"
    );
    assert!(host.current.is_none());
}

#[test]
fn the_game_ending_by_itself_finishes_it_and_unproven_settlement_never_does() {
    let root = tempfile::tempdir().unwrap();
    let mut ended = started(root.path(), GameState::ExitedWithoutResult);
    assert!(matches!(ended.poll(), Progress::Finished { .. }));

    let mut stuck = started(root.path(), GameState::ExitedWithoutResult);
    stuck.port.stop_fails = true;
    assert!(matches!(
        stuck.poll(),
        Progress::Failed { code, .. } if code == "SettlementUnverified"
    ));
}

#[test]
fn a_state_that_voids_the_launch_is_handled_as_it_is_for_a_measured_attempt() {
    let root = tempfile::tempdir().unwrap();
    let mut host = started(root.path(), GameState::GpuReset);
    assert!(matches!(
        host.poll(),
        Progress::Failed { code, .. } if code == "GpuReset"
    ));
}
