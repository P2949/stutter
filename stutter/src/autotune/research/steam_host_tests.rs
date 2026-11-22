use super::*;
use crate::autotune::research::ProtonSource;

struct Port {
    safe: bool,
    states: Vec<GameState>,
    launches: u32,
    stops: u32,
    tool_missing: bool,
    /// Settlement steps that still report `Pending` before the tree is proven gone.
    pending: u32,
}

impl SteamPort for Port {
    fn observe_desktop_idle(&mut self, _: Option<std::time::Instant>) {}
    fn observe_user_input_idle(&mut self, _: Option<std::time::Instant>) {}
    fn safe_window(&mut self) -> bool {
        self.safe
    }
    fn tool_dir(&mut self, _: &AttemptSlot) -> anyhow::Result<PathBuf> {
        anyhow::ensure!(!self.tool_missing, "tool disappeared");
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
    fn stop_step(&mut self) -> StopStep {
        if self.pending > 0 {
            self.pending -= 1;
            return StopStep::Pending;
        }
        self.stops += 1;
        StopStep::Settled
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
        tool_missing: false,
        pending: 0,
    })
}

#[test]
fn a_missing_scheduled_tool_is_an_integrity_failure_during_arming() {
    let mut host = host(vec![]);
    host.port.tool_missing = true;
    let error = host.arm_spec(&slot()).unwrap_err();
    assert_eq!(
        error
            .downcast_ref::<super::super::driver_failure::LaunchFailure>()
            .unwrap()
            .class,
        super::super::driver_failure::FailureClass::Integrity,
    );
    assert_eq!(host.port.launches, 0);
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
fn a_result_carries_the_temperatures_at_launch_and_at_the_result() {
    let hwmon = tempfile::tempdir().unwrap();
    let write = |cpu: &str, gpu: &str| {
        for (index, (name, value)) in [("coretemp", cpu), ("amdgpu", gpu)].into_iter().enumerate() {
            let dir = hwmon.path().join(format!("hwmon{index}"));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("name"), name).unwrap();
            std::fs::write(dir.join("temp1_input"), value).unwrap();
        }
    };
    write("50000", "45000");
    let mut host = host(vec![GameState::Result {
        completion: Box::new(completion()),
    }]);
    host.hwmon_root = hwmon.path().to_path_buf();
    launch(&mut host);
    write("73000", "64000");
    let Progress::Finished { evidence } = host.poll() else {
        panic!("expected a result")
    };
    let thermal = evidence.thermal.expect("a recorded thermal context");
    assert_eq!(thermal.start.cpu_mc, Some(50_000));
    assert_eq!(thermal.end.cpu_mc, Some(73_000));
    assert_eq!(
        (thermal.start.gpu_mc, thermal.end.gpu_mc),
        (Some(45_000), Some(64_000))
    );
}

#[test]
fn exit_without_a_native_result_and_identity_mismatch_fail_closed() {
    let mut host = host(vec![GameState::ExitedWithoutResult]);
    launch(&mut host);
    assert_eq!(
        host.poll(),
        Progress::Failed {
            class: crate::autotune::research::driver::FailureClass::Incompatible,
            code: "ExitedWithoutNativeResult".into()
        }
    );
    let mut host = host_with_mismatch();
    launch(&mut host);
    assert_eq!(
        host.poll(),
        Progress::Failed {
            class: crate::autotune::research::driver::FailureClass::Integrity,
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
            class: crate::autotune::research::driver::FailureClass::Environment,
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
            class: crate::autotune::research::driver::FailureClass::Integrity,
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
        host.poll()
    };
    let binding = |progress| match progress {
        Progress::Finished { evidence } => evidence.native_completion.unwrap().runtime_binding,
        other => panic!("expected a completed result, got {other:?}"),
    };
    // The arm's installation is /tools/GE.
    assert_eq!(
        binding(binding_for(Some(runtime("/tools/GE/files")))),
        Some(RuntimeBinding::Matched)
    );
    assert_eq!(
        binding(binding_for(Some(runtime("/run/host/tools/GE/files")))),
        Some(RuntimeBinding::Matched)
    );
    assert!(matches!(
        binding_for(Some(runtime("/tools/CachyOS/files"))),
        Progress::Failed {
            class: crate::autotune::research::driver::FailureClass::Integrity,
            ..
        }
    ));
    // Nothing captured proves nothing and records no binding.
    assert_eq!(binding(binding_for(None)), None);
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
    // Off runs the catalog's explicit control; On carries the factor; same build.
    assert_eq!(
        host.arm_spec(&at(0)).unwrap().arm_env["PROTON_ENABLE_WAYLAND"],
        "0"
    );
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
    // NTSync: the control arm disables it, so the two arms never share an environment.
    let mut ge = slot();
    ge.arm = ProtonSource::Ge;
    host.set_enabled_factors(&[ResearchFactor::NtSync]);
    host.set_treatments(Some(vec![Treatment::Off, Treatment::On]));
    let ge_at = |index: u32| AttemptSlot {
        index,
        ..ge.clone()
    };
    let off = host.arm_spec(&ge_at(0)).unwrap().arm_env;
    let on = host.arm_spec(&ge_at(1)).unwrap().arm_env;
    assert_eq!(off["PROTON_NO_NTSYNC"], "1");
    assert_ne!(off, on);
}

#[test]
fn a_slow_settlement_keeps_the_decided_outcome_and_the_attempt_running_without_blocking() {
    let mut host = host(vec![GameState::ExitedWithoutResult]);
    launch(&mut host);
    host.port.pending = 2;
    // The game ended; the tree is not yet proven gone: each poll is one step, nothing is reported, and the port's
    // state is not read again (it would panic: only one state was scripted).
    assert!(matches!(host.poll(), Progress::Running));
    assert!(matches!(host.poll(), Progress::Running));
    assert!(host.current.is_some(), "ownership is kept while settling");
    let Progress::Failed { code, .. } = host.poll() else {
        panic!("the decided outcome is delivered once settled");
    };
    assert_eq!(code, "ExitedWithoutNativeResult");
    assert!(host.current.is_none());
}

#[test]
fn a_settlement_that_cannot_be_proven_fails_the_attempt_as_unresolved_ownership() {
    struct Stuck(Port);
    impl SteamPort for Stuck {
        fn observe_desktop_idle(&mut self, a: Option<std::time::Instant>) {
            self.0.observe_desktop_idle(a)
        }
        fn observe_user_input_idle(&mut self, a: Option<std::time::Instant>) {
            self.0.observe_user_input_idle(a)
        }
        fn safe_window(&mut self) -> bool {
            self.0.safe_window()
        }
        fn tool_dir(&mut self, s: &AttemptSlot) -> anyhow::Result<PathBuf> {
            self.0.tool_dir(s)
        }
        fn study_compat_data(&self) -> PathBuf {
            self.0.study_compat_data()
        }
        fn library_paths(&self) -> (PathBuf, PathBuf) {
            self.0.library_paths()
        }
        fn launch(&mut self, s: &AttemptSlot, c: &DispatchContextV1) -> anyhow::Result<()> {
            self.0.launch(s, c)
        }
        fn state(&mut self) -> GameState {
            self.0.state()
        }
        fn stop(&mut self) -> anyhow::Result<()> {
            anyhow::bail!("never settles")
        }
        fn stop_step(&mut self) -> StopStep {
            StopStep::Unsettled
        }
    }
    let mut host = SteamAttemptHost::new(Stuck(Port {
        safe: true,
        states: vec![GameState::IdentityMismatch],
        launches: 0,
        stops: 0,
        tool_missing: false,
        pending: 0,
    }));
    let spec = host.arm_spec(&slot()).unwrap();
    let context = DispatchContextV1 {
        schema_version: 1,
        intent: crate::autotune::research::tests::intent(),
        slot: slot(),
        tickets: Default::default(),
        spec,
    };
    host.launch(&slot(), &context).unwrap();
    let Progress::Failed { class, code } = host.poll() else {
        panic!("an unproven settlement is a failure");
    };
    assert_eq!(code, "SettlementUnverified");
    assert_eq!(
        class,
        crate::autotune::research::driver::FailureClass::OwnershipUnresolved
    );
}
