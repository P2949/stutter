use std::{
    collections::BTreeMap,
    fs,
    os::unix::net::UnixListener,
    path::{Path, PathBuf},
    sync::Arc,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use stutter::api::{
    actions::{
        ActionId, GpuPowerAction, IrqAffinityAction, IrqAffinityEvidence, IrqAffinityRisk,
        NiceAction, NicePolicy, SafetyClass, TaskIdentity, VmKnobAction, VmKnobChange,
    },
    autotune::{
        ObjectiveKind,
        activity::ActivityLevel,
        candidate::{
            CandidateAction, CandidateEvidence, CandidatePlanFile, GpuPowerActionPlan,
            IrqAffinityActionPlan, NiceActionPlan, VmKnobActionPlan,
        },
        controller::ControllerRuntimeState,
        controller_journal::read_controller_journal,
        emergency_restore::{
            AutotuneRestoreCommandInput, AutotuneRestoreStatus, restore_known_autotune_actions,
        },
        observation::{AutotuneObservation, OnlineDataQuality},
        planner::{CandidateDenyReason, CandidatePlanner, PlannerInput},
        providers::{
            CandidateProposal, CandidateProvider, CandidateProviderInput,
            CandidateProviderRegistry, VmKnobProvider,
        },
        runtime::{AutotuneRuntime, AutotuneRuntimeConfig, daemon_config_for_runtime_mode},
        state::{ControllerPhase, SituationKind},
        system_context::SystemContextSnapshot,
        workload_policy::WorkloadPolicyMatrix,
    },
    daemon::{
        ActionSource, DaemonCapabilities, DaemonMode, DaemonPolicy,
        InProcessPrivilegedActionService, SystemHealthSnapshot, UnixSocketPrivilegedActionService,
        run_privileged_worker_with_service,
    },
    focus::FocusGroupKind,
    process_tree::{TaskClass, TaskInfo},
    session_events::{DropCountersSnapshot, IntervalRecord, MonitorEvent},
};

fn temp_dir() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before Unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "stutter-autotune-lifecycle-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("failed to create lifecycle temp dir");
    path
}

fn wait_for_socket(path: &Path) {
    for _ in 0..200 {
        if path.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!("timed out waiting for socket {}", path.display());
}

fn unix_socket_bind_supported() -> bool {
    let dir = temp_dir();
    let path = dir.join("support-probe.sock");
    match UnixListener::bind(&path) {
        Ok(listener) => {
            drop(listener);
            fs::remove_file(path).ok();
            true
        }
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => false,
        Err(err) => panic!("unexpected lifecycle unix socket probe error: {err}"),
    }
}

fn parse_thread_stat() -> anyhow::Result<(u32, i32, u64)> {
    let stat = fs::read_to_string("/proc/thread-self/stat")?;
    let tid = stat
        .split_once(' ')
        .and_then(|(tid, _)| tid.parse::<u32>().ok())
        .ok_or_else(|| anyhow::anyhow!("failed to parse thread tid from stat"))?;
    let close_paren = stat
        .rfind(')')
        .ok_or_else(|| anyhow::anyhow!("stat line does not contain closing comm parenthesis"))?;
    let fields = stat[close_paren + 1..]
        .split_whitespace()
        .collect::<Vec<_>>();
    let nice = fields
        .get(16)
        .ok_or_else(|| anyhow::anyhow!("stat line missing nice field"))?
        .parse::<i32>()?;
    let starttime_ticks = fields
        .get(19)
        .ok_or_else(|| anyhow::anyhow!("stat line missing starttime field"))?
        .parse::<u64>()?;

    Ok((tid, nice, starttime_ticks))
}

fn current_thread_nice_candidate() -> anyhow::Result<(CandidateAction, u32)> {
    let (tid, nice, starttime_ticks) = parse_thread_stat()?;
    let comm = fs::read_to_string("/proc/thread-self/comm")
        .ok()
        .map(|comm| comm.trim().to_owned())
        .filter(|comm| !comm.is_empty());
    let target = TaskIdentity {
        tid: tid.into(),
        process_pid: Some(std::process::id().into()),
        comm,
        starttime_ticks: Some(starttime_ticks),
    };

    Ok((
        CandidateAction::Nice {
            plan: NiceActionPlan {
                name: "medium-risk-socket-nice-game-noop".to_owned(),
                action: NiceAction {
                    targets: vec![target],
                    nice,
                    policy: NicePolicy::default(),
                },
                target_root_pid: Some(std::process::id()),
                evidence: vec![CandidateEvidence::new(
                    "test_current_thread",
                    format!("tid={tid} nice={nice}"),
                    1.0,
                )],
                objective: ObjectiveKind::GameRunnableLatency,
            },
        },
        tid,
    ))
}

fn game_task(tid: u32) -> TaskInfo {
    TaskInfo {
        tid: tid.into(),
        process_pid: 1234.into(),
        process_ppid: 1.into(),
        comm: "lifecycle-game".to_owned(),
        process_comm: "lifecycle-game".into(),
        process_starttime_ticks: Some(10_000),
        task_starttime_ticks: Some(10_000 + u64::from(tid)),
        exe_dev: Some(1),
        exe_ino: Some(1234),
        class: TaskClass::Game,
        sched_policy: Some(0),
        from_cgroup: false,
    }
}

fn current_process_game_task(tid: u32) -> TaskInfo {
    TaskInfo {
        tid: tid.into(),
        process_pid: std::process::id().into(),
        process_ppid: 1.into(),
        comm: "lifecycle-game".to_owned(),
        process_comm: "lifecycle-game".into(),
        process_starttime_ticks: None,
        task_starttime_ticks: None,
        exe_dev: Some(1),
        exe_ino: Some(1234),
        class: TaskClass::Game,
        sched_policy: Some(0),
        from_cgroup: false,
    }
}

fn record(
    elapsed_ms: u64,
    samples: u64,
    over_1ms: u64,
    over_2ms: u64,
    over_5ms: u64,
    max_ns: u64,
) -> IntervalRecord {
    IntervalRecord {
        elapsed_ms,
        task: 1234,
        active: true,
        class: TaskClass::Game,
        comm: "lifecycle-game".to_owned(),
        process_pid: Some(1234),
        process_comm: "lifecycle-game".into(),
        samples,
        stored_samples: samples,
        max_ns,
        over_1ms,
        over_2ms,
        over_5ms,
        percentile_scope: "task".to_owned(),
        ..IntervalRecord::default()
    }
}

#[derive(Clone, Copy)]
struct RecordLatency {
    samples: u64,
    over_1ms: u64,
    over_2ms: u64,
    over_5ms: u64,
    max_ns: u64,
}

fn record_for_task(
    elapsed_ms: u64,
    task: u32,
    process_pid: u32,
    lat: RecordLatency,
) -> IntervalRecord {
    IntervalRecord {
        elapsed_ms,
        task,
        active: true,
        class: TaskClass::Game,
        comm: "lifecycle-game".to_owned(),
        process_pid: Some(process_pid),
        process_comm: "lifecycle-game".into(),
        samples: lat.samples,
        stored_samples: lat.samples,
        max_ns: lat.max_ns,
        over_1ms: lat.over_1ms,
        over_2ms: lat.over_2ms,
        over_5ms: lat.over_5ms,
        percentile_scope: "task".to_owned(),
        ..IntervalRecord::default()
    }
}

fn records(
    start_elapsed_ms: u64,
    count: usize,
    samples: u64,
    over_1ms: u64,
    over_2ms: u64,
    over_5ms: u64,
    max_ns: u64,
) -> Vec<IntervalRecord> {
    (0..count)
        .map(|offset| {
            record(
                start_elapsed_ms + (offset as u64 * 1_000),
                samples,
                over_1ms,
                over_2ms,
                over_5ms,
                max_ns,
            )
        })
        .collect()
}

fn records_for_task(
    start_elapsed_ms: u64,
    count: usize,
    task: u32,
    process_pid: u32,
    lat: RecordLatency,
) -> Vec<IntervalRecord> {
    (0..count)
        .map(|offset| {
            record_for_task(
                start_elapsed_ms + (offset as u64 * 1_000),
                task,
                process_pid,
                lat,
            )
        })
        .collect()
}

fn interval_event(records: Vec<IntervalRecord>) -> MonitorEvent {
    let elapsed_ms = records.last().map(|record| record.elapsed_ms).unwrap_or(0);
    MonitorEvent::Interval {
        elapsed_ms,
        records,
        drop_counters: DropCountersSnapshot::default(),
    }
}

#[test]
fn irq_affinity_gpu_device_is_medium_risk_not_manual_only() {
    let evidence = IrqAffinityEvidence {
        strong_irq_evidence: true,
        stable_irq_identity: true,
        known_device_mapping: true,
        observed_irq: Some(42),
        observed_device_hint: Some("amdgpu".to_owned()),
        reason: "integration test GPU IRQ evidence".to_owned(),
    };
    let candidate = CandidateAction::IrqAffinity {
        plan: IrqAffinityActionPlan {
            name: "irq-gpu-medium".to_owned(),
            action: IrqAffinityAction::new(
                42,
                "amdgpu".to_owned(),
                "1".to_owned(),
                IrqAffinityRisk::ReversibleMediumRisk,
                evidence,
            ),
            evidence: vec![CandidateEvidence::new("irq_device", "amdgpu", 1.0)],
            objective: ObjectiveKind::IrqOverlapReduction,
        },
    };

    assert!(!candidate.is_high_risk_system_adjacent());
    assert!(candidate.manual_only_reason().is_none());
    assert_eq!(candidate.safety_class(), SafetyClass::ReversibleMediumRisk);
    assert!(
        CandidatePlanFile::from_candidate(&candidate, None)
            .executable
            .is_some()
    );
}

#[test]
fn gpu_power_profile_switch_only_is_medium_risk() {
    let candidate = CandidateAction::GpuPower {
        plan: GpuPowerActionPlan {
            name: "gpu-profile-medium".to_owned(),
            action: GpuPowerAction {
                sysfs_root: PathBuf::from("/sys"),
                drm_card: "card0".to_owned(),
                power_dpm_force_performance_level: None,
                pp_power_profile_mode: Some("3D_FULL_SCREEN".to_owned()),
            },
            evidence: vec![CandidateEvidence::new("gpu_profile", "card0", 1.0)],
            objective: ObjectiveKind::GameFramePacing,
        },
    };

    assert!(!candidate.is_high_risk_system_adjacent());
    assert_eq!(candidate.safety_class(), SafetyClass::ReversibleMediumRisk);
    assert!(
        CandidatePlanFile::from_candidate(&candidate, None)
            .executable
            .is_some()
    );
}

#[test]
fn vm_swappiness_candidate_is_medium_risk_and_executable() {
    let candidate = CandidateAction::VmKnob {
        plan: VmKnobActionPlan {
            name: "vm-swappiness-medium".to_owned(),
            action: VmKnobAction {
                root: PathBuf::from("/"),
                changes: vec![VmKnobChange {
                    path: PathBuf::from("proc/sys/vm/swappiness"),
                    value: "10".to_owned(),
                }],
            },
            evidence: vec![CandidateEvidence::new(
                "swap_pressure",
                "mem_stall_spike",
                1.0,
            )],
            objective: ObjectiveKind::DesktopInteractivity,
        },
    };

    assert!(!candidate.is_high_risk_system_adjacent());
    assert_eq!(candidate.safety_class(), SafetyClass::ReversibleMediumRisk);
    assert!(
        CandidatePlanFile::from_candidate(&candidate, None)
            .executable
            .is_some()
    );
}

#[test]
fn vm_swappiness_provider_produces_medium_risk_non_manual_candidate() {
    let provider = VmKnobProvider;
    let mut observation = AutotuneObservation {
        target_present: true,
        target_root_pid: Some(1234),
        active_target_count: 1,
        scored_task_count: 1,
        interval_count: 3,
        scored_samples: 300,
        data_quality: OnlineDataQuality::High,
        primary_situation: SituationKind::IoPressure,
        focus_kind: Some(FocusGroupKind::Desktop),
        focus_confidence: 0.95,
        ..AutotuneObservation::default()
    };
    observation.refresh_situation_classification();
    observation.primary_situation = SituationKind::IoPressure;
    observation.situation.confidence = 0.95;
    observation.objective_signals.swap_activity_events = Some(100);

    let mut system_context = SystemContextSnapshot::from_observation(&observation);
    system_context
        .inventory
        .vm_knobs
        .insert("proc/sys/vm/swappiness".to_owned(), "60".to_owned());
    let policy = DaemonPolicy::suggest(ActionSource::Test);
    let controller_state = ControllerRuntimeState::default();

    let proposals = provider.propose(&CandidateProviderInput {
        observation: &observation,
        daemon_policy: &policy,
        capabilities: &observation.capabilities,
        system_health: &observation.system_health,
        system_context: &system_context,
        controller_state: &controller_state,
        profiles: &[],
    });

    let swappiness = proposals
        .iter()
        .find(|proposal| match &proposal.candidate {
            CandidateAction::VmKnob { plan } => plan
                .action
                .changes
                .iter()
                .any(|change| change.path == *Path::new("proc/sys/vm/swappiness")),

            _ => false,
        })
        .expect("vm.swappiness proposal should be produced for swap pressure");

    assert_eq!(
        swappiness.candidate.safety_class(),
        SafetyClass::ReversibleMediumRisk
    );
    assert!(!swappiness.candidate.is_high_risk_system_adjacent());
    assert!(swappiness.candidate.manual_only_reason().is_none());
}

struct StaticCandidateProvider {
    candidate: CandidateAction,
}

impl CandidateProvider for StaticCandidateProvider {
    fn family(&self) -> &'static str {
        self.candidate.action_kind()
    }

    fn propose(&self, _: &CandidateProviderInput<'_>) -> Vec<CandidateProposal> {
        vec![CandidateProposal {
            candidate: self.candidate.clone(),
            provider: self.family(),
            confidence: 1.0,
            deny_reasons: Vec::new(),
            objective: ObjectiveKind::GameRunnableLatency,
            rank_hint: 0,
        }]
    }
}

#[test]
fn activity_classifier_suppresses_idle_game_candidates() {
    let candidate = CandidateAction::fake(
        ActionId::new("idle-suppressed-fake".to_owned()),
        SafetyClass::ReversibleLowRisk,
    );
    let mut registry = CandidateProviderRegistry::default();
    registry.register(Box::new(StaticCandidateProvider { candidate }));
    let planner = CandidatePlanner::new(registry);
    let mut observation = AutotuneObservation {
        target_present: true,
        target_root_pid: Some(1234),
        active_target_count: 1,
        scored_task_count: 1,
        interval_count: 5,
        scored_samples: 0,
        data_quality: OnlineDataQuality::High,
        activity_level: ActivityLevel::Idle,
        primary_situation: SituationKind::GameCpuSchedulerPressure,
        focus_kind: Some(FocusGroupKind::Game),
        focus_confidence: 0.95,
        focus_roots: vec![1234],
        ..AutotuneObservation::default()
    };
    observation.refresh_situation_classification();
    observation.primary_situation = SituationKind::GameCpuSchedulerPressure;
    let policy = DaemonPolicy::suggest(ActionSource::Test);
    let workload_policy = WorkloadPolicyMatrix::default_rules();
    let controller_state = ControllerRuntimeState::default();
    let capabilities = DaemonCapabilities::default();
    let system_health = SystemHealthSnapshot::default();

    let result = planner.plan(PlannerInput {
        observation: &observation,
        daemon_policy: &policy,
        capabilities: &capabilities,
        system_health: &system_health,
        controller_state: &controller_state,
        active_profile_state: None,
        workload_policy: &workload_policy,
        profiles: &[],
    });

    assert!(result.selected.is_none());
    assert!(result.evaluations.iter().any(|evaluation| {
        evaluation
            .deny_reasons
            .contains(&CandidateDenyReason::WorkloadIdle)
    }));
}

#[tokio::test(flavor = "current_thread")]
async fn medium_risk_apply_through_unix_socket_lifecycle() -> anyhow::Result<()> {
    if !unix_socket_bind_supported() {
        return Ok(());
    }

    let dir = temp_dir();
    let history_path = dir.join("autotune-history.jsonl");
    let audit_path = dir.join("audit.jsonl");
    let journal_path = dir.join("controller-journal.json");
    let socket_path = dir.join("privileged-worker.sock");

    let worker_service = Arc::new(InProcessPrivilegedActionService::default());
    let worker_socket_path = socket_path.clone();
    let worker_service_for_thread = Arc::clone(&worker_service);
    let worker_thread = thread::spawn(move || {
        run_privileged_worker_with_service(&worker_socket_path, worker_service_for_thread.as_ref())
    });
    wait_for_socket(&socket_path);

    let (candidate, tid) = current_thread_nice_candidate()?;
    assert_eq!(candidate.safety_class(), SafetyClass::ReversibleMediumRisk);

    let root_pid = std::process::id();
    let mut daemon_config = daemon_config_for_runtime_mode(
        DaemonMode::ApplyMediumRisk,
        ActionSource::AutotuneRuntime,
        Some(root_pid),
        None,
    );
    daemon_config.autotune.allow_medium_risk_apply = true;
    daemon_config.autotune.manage_privileged_worker = false;
    daemon_config.autotune.unsafe_in_process_privileged_worker = false;
    daemon_config.autotune.privileged_worker_socket = Some(socket_path.clone());

    let mut config = AutotuneRuntimeConfig::from_daemon_config(daemon_config, None)
        .with_simulated_candidates(vec![candidate])
        .with_candidate_window_seconds(1)
        .with_washout(0, 1);
    config.history_log = Some(history_path.clone());
    config.controller_journal_path = Some(journal_path.clone());

    let mut runtime = AutotuneRuntime::new(config);
    let mut active_targets = BTreeMap::new();
    active_targets.insert(root_pid.into(), current_process_game_task(tid));

    runtime.on_event(MonitorEvent::TargetSnapshot {
        elapsed_ms: 0,
        active_targets,
        removed_targets: Vec::new(),
    })?;
    runtime.on_event(MonitorEvent::FocusChanged {
        elapsed_ms: 0,
        old_kind: None,
        new_kind: FocusGroupKind::Game,
        root_pids: vec![root_pid],
        member_pids: vec![root_pid],
        confidence: 0.95,
        score: 1.0,
        situation: SituationKind::GameCpuSchedulerPressure,
        reasons: vec!["medium-risk socket lifecycle test game focus".to_owned()],
    })?;

    runtime.on_event(interval_event(records_for_task(
        1_000,
        4,
        tid,
        root_pid,
        RecordLatency {
            samples: 25,
            over_1ms: 5,
            over_2ms: 5,
            over_5ms: 2,
            max_ns: 8_000_000,
        },
    )))?;
    assert_eq!(runtime.controller_state().phase, ControllerPhase::Observing);

    let started = runtime
        .on_event(interval_event(records_for_task(
            5_000,
            1,
            tid,
            root_pid,
            RecordLatency {
                samples: 25,
                over_1ms: 5,
                over_2ms: 5,
                over_5ms: 2,
                max_ns: 8_000_000,
            },
        )))?
        .expect("baseline window should start a medium-risk experiment through the socket");
    assert_eq!(started.decision, "candidate_started");
    assert_eq!(runtime.controller_state().phase, ControllerPhase::Measuring);
    assert!(runtime.has_active_experiment());

    thread::sleep(Duration::from_millis(2_100));
    let kept = runtime
        .on_event(interval_event(records_for_task(
            6_000,
            5,
            tid,
            root_pid,
            RecordLatency {
                samples: 25,
                over_1ms: 0,
                over_2ms: 0,
                over_5ms: 0,
                max_ns: 500_000,
            },
        )))?
        .expect("candidate measurement should keep the improved medium-risk action");
    assert_eq!(kept.decision, "candidate_kept");
    assert_eq!(runtime.controller_state().phase, ControllerPhase::Cooldown);
    assert_eq!(runtime.active_profile_state().kept_action_count(), 1);

    assert_ne!(runtime.controller_state().phase, ControllerPhase::Faulted);
    assert!(history_path.exists());
    assert!(!fs::read_to_string(&history_path)?.trim().is_empty());

    let restore = restore_known_autotune_actions(AutotuneRestoreCommandInput {
        journal_path: Some(journal_path.clone()),
        audit_path: Some(audit_path),
        history_path: Some(history_path),
        dry_run: false,
    })?;
    assert_eq!(restore.status, AutotuneRestoreStatus::Restored);
    assert!(read_controller_journal(&journal_path)?.is_clean());

    UnixSocketPrivilegedActionService::new(&socket_path).request_shutdown()?;
    worker_thread.join().expect("worker thread panicked")?;
    assert!(!socket_path.exists());

    Ok(())
}

#[tokio::test]
async fn apply_low_risk_fake_candidate_lifecycle_keeps_and_cleans_journal() -> anyhow::Result<()> {
    let dir = temp_dir();
    let history_path = dir.join("autotune-history.jsonl");
    let audit_path = dir.join("audit.jsonl");
    let journal_path = dir.join("controller-journal.json");

    let candidate = CandidateAction::fake(
        ActionId::new("test-fake".to_owned()),
        SafetyClass::ReversibleLowRisk,
    );
    let mut config = AutotuneRuntimeConfig::apply_low_risk(None, Some(1234), None)
        .with_simulated_candidates(vec![candidate])
        .with_simulated_action_effects()
        .with_candidate_window_seconds(1)
        .with_washout(0, 1);
    config.history_log = Some(history_path.clone());
    config.controller_journal_path = Some(journal_path.clone());

    let mut runtime = AutotuneRuntime::new(config);
    let mut active_targets = BTreeMap::new();
    active_targets.insert(1234.into(), game_task(1234));

    runtime.on_event(MonitorEvent::TargetSnapshot {
        elapsed_ms: 0,
        active_targets,
        removed_targets: Vec::new(),
    })?;
    runtime.on_event(MonitorEvent::FocusChanged {
        elapsed_ms: 0,
        old_kind: None,
        new_kind: FocusGroupKind::Game,
        root_pids: vec![1234],
        member_pids: vec![1234],
        confidence: 0.95,
        score: 1.0,
        situation: SituationKind::GameCpuSchedulerPressure,
        reasons: vec!["lifecycle test game focus".to_owned()],
    })?;

    runtime.on_event(interval_event(records(1_000, 4, 25, 5, 5, 2, 8_000_000)))?;
    assert_eq!(runtime.controller_state().phase, ControllerPhase::Observing);

    let started = runtime
        .on_event(interval_event(records(5_000, 1, 25, 5, 5, 2, 8_000_000)))?
        .expect("baseline window should start a fake experiment");
    assert_eq!(started.decision, "candidate_started");
    assert_eq!(runtime.controller_state().phase, ControllerPhase::Measuring);
    assert!(runtime.has_active_experiment());

    let kept = runtime
        .on_event(interval_event(records(6_000, 5, 25, 0, 0, 0, 500_000)))?
        .expect("candidate measurement should keep the improved fake action");
    assert_eq!(kept.decision, "candidate_kept");
    assert_eq!(runtime.controller_state().phase, ControllerPhase::Cooldown);
    assert_eq!(runtime.active_profile_state().kept_action_count(), 1);

    for tick in 0..53 {
        runtime.on_event(interval_event(records(
            12_000 + (tick * 1_000),
            1,
            25,
            0,
            0,
            0,
            500_000,
        )))?;
    }

    assert_ne!(runtime.controller_state().phase, ControllerPhase::Faulted);
    assert!(history_path.exists());
    assert!(!fs::read_to_string(&history_path)?.trim().is_empty());

    let restore = restore_known_autotune_actions(AutotuneRestoreCommandInput {
        journal_path: Some(journal_path.clone()),
        audit_path: Some(audit_path),
        history_path: Some(history_path),
        dry_run: false,
    })?;
    assert_eq!(restore.status, AutotuneRestoreStatus::Restored);
    assert!(read_controller_journal(&journal_path)?.is_clean());

    Ok(())
}
