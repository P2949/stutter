use serde::Deserialize;

pub(super) use super::super::{
    super::{CandidateDenyReason, CandidatePlanner, PlannerInput},
    support::*,
};
use crate::autotune::activity::ActivityLevel;
#[derive(Debug, Deserialize)]
pub(super) struct PlannerGoldenCase {
    pub(super) situation: String,
    pub(super) focus_kind: String,
    pub(super) policy: DaemonPolicyFixture,
    pub(super) expected_selected_action_kind: Option<String>,
    pub(super) expected_total_proposals: usize,
    pub(super) expected_eligible_proposals: usize,
    pub(super) expected_evaluations: Vec<ExpectedEvaluation>,
    #[serde(default)]
    pub(super) low_data_quality: bool,
    #[serde(default)]
    pub(super) critical_realtime: bool,
    #[serde(default)]
    pub(super) cooldown_active: bool,
    #[serde(default)]
    pub(super) kept_conflict: bool,
    #[serde(default)]
    pub(super) external_mutation: bool,
    #[serde(default)]
    pub(super) cpu_power_evidence: bool,
    #[serde(default)]
    pub(super) gpu_power_evidence: bool,
    #[serde(default)]
    pub(super) irq_evidence: bool,
    #[serde(default)]
    pub(super) vm_evidence: bool,
    #[serde(default)]
    pub(super) thermal_degraded: bool,
    #[serde(default)]
    pub(super) activity_level: Option<ActivityLevel>,
    // Optional ObjectiveSignals override for fixtures that exercise the
    // rolling-window/observation signal path directly.
    #[serde(default)]
    pub(super) hardware_signals: Option<serde_json::Value>,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct DaemonPolicyFixture {
    pub(super) mode: String,
    #[serde(default)]
    pub(super) allow_system_wide_suggestions: bool,
    #[serde(default)]
    pub(super) allow_medium_risk_apply: bool,
    #[serde(default)]
    pub(super) enabled_action_families: Vec<String>,
    #[serde(default)]
    pub(super) irq_devices: Vec<String>,
    #[serde(default)]
    pub(super) gpu_cards: Vec<String>,
    #[serde(default)]
    pub(super) allow_gpu_power_in_autotune: bool,
    #[serde(default)]
    pub(super) vm_knobs: Vec<String>,
    #[serde(default)]
    pub(super) allow_vm_knobs_in_autotune: bool,
    #[serde(default)]
    pub(super) autonomous_families: Vec<String>,
    #[serde(default)]
    pub(super) compile_cgroup: Option<String>,
    #[serde(default)]
    pub(super) background_cgroup: Option<String>,
    #[serde(default)]
    pub(super) game_cgroup: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ExpectedEvaluation {
    pub(super) action_kind: String,
    pub(super) objective: String,
    pub(super) eligible: bool,
    pub(super) min_confidence: f32,
    pub(super) max_confidence: f32,
    pub(super) dry_run_affected_tasks: Option<usize>,
    pub(super) manual_only: bool,
    pub(super) deny_reason_codes: Vec<String>,
}
pub(super) fn build_fixture_policy(
    fixture: &DaemonPolicyFixture,
    tree_pid: Option<u32>,
) -> DaemonPolicy {
    let mode = fixture.mode.parse::<DaemonMode>().unwrap();
    let mut config = crate::autotune::runtime::daemon_config_for_runtime_mode(
        mode,
        ActionSource::AutotuneRuntime,
        tree_pid,
        None,
    );
    config.safety.allow_system_wide_suggestions = fixture.allow_system_wide_suggestions;
    config.autotune.allow_medium_risk_apply = fixture.allow_medium_risk_apply;
    config.safety.enabled_action_families =
        fixture.enabled_action_families.iter().cloned().collect();
    config.safety.system_wide_allowlist.irq_devices = fixture.irq_devices.iter().cloned().collect();
    config.safety.system_wide_allowlist.gpu_cards = fixture.gpu_cards.iter().cloned().collect();
    config.safety.system_wide_allowlist.vm_knobs = fixture.vm_knobs.iter().cloned().collect();
    config.autotune.allow_gpu_power_in_autotune = fixture.allow_gpu_power_in_autotune;
    config.autotune.allow_vm_knobs_in_autotune = fixture.allow_vm_knobs_in_autotune;

    if let Some(path) = &fixture.compile_cgroup {
        config.safety.cgroup_targets.compile_cgroup = Some(std::path::PathBuf::from(path));
    }
    if let Some(path) = &fixture.background_cgroup {
        config.safety.cgroup_targets.background_cgroup = Some(std::path::PathBuf::from(path));
    }
    if let Some(path) = &fixture.game_cgroup {
        config.safety.cgroup_targets.game_cgroup = Some(std::path::PathBuf::from(path));
    }

    build_daemon_policy(DaemonPolicyBuildInput {
        config: &config,
        remote_context: None,
    })
}

pub(super) fn fixture_workload_policy(case: &PlannerGoldenCase) -> WorkloadPolicyMatrix {
    let mut policy = WorkloadPolicyMatrix::default_rules();
    if !case.policy.autonomous_families.is_empty() {
        let situation = parse_fixture_situation(&case.situation);
        if let Some(rule) = policy
            .rules
            .iter_mut()
            .find(|rule| rule.situation == situation)
        {
            rule.autonomous_families = case.policy.autonomous_families.iter().cloned().collect();
        }
    }
    policy
}

pub(super) fn build_fixture_observation(case: &PlannerGoldenCase) -> AutotuneObservation {
    let situation = parse_fixture_situation(&case.situation);
    let focus_kind = parse_fixture_focus_kind(&case.focus_kind);
    let mut observation = observation_for_situation(situation, focus_kind);

    observation.focus_confidence = 0.95;
    observation.situation.primary = situation;
    observation.situation.confidence = 0.95;
    if let Some(activity_level) = case.activity_level {
        observation.activity_level = activity_level;
    }
    observation.active_tasks = fixture_tasks(situation, focus_kind);
    observation.capabilities = DaemonCapabilities {
        btf_available: true,
        sched_tracepoints_available: true,
        perf_permissions_likely: true,
        cgroup_v2_available: true,
        sched_ext_available: true,
        uclamp_available: true,
        ionice_available: true,
        irq_affinity_available: true,
        gpu_sysfs_available: true,
        ..DaemonCapabilities::default()
    };
    observation.system_health = SystemHealthSnapshot {
        ok_for_apply: true,
        ..SystemHealthSnapshot::default()
    };

    if case.low_data_quality {
        observation.data_quality = OnlineDataQuality::Low {
            reasons: vec!["fixture low data quality".to_owned()],
        };
    }

    if case.critical_realtime {
        observation
            .focus_reasons
            .push("critical realtime input process present".to_owned());
    }

    if case.thermal_degraded {
        observation.objective_signals.thermal_degraded = Some(true);
        observation.objective_signals.thermal_throttle_count = Some(1);
        observation.objective_signals.signal_quality.thermal = ObjectiveSignalQuality::Direct;
    }

    let mut inventory = crate::system_inventory::SystemInventory {
        cpu_policies: Vec::new(),
        drm_devices: Vec::new(),
        irq_default_smp_affinity: Some("f".to_owned()),
        irq_lines: Vec::new(),
        power_source: crate::system_inventory::PowerSourceSnapshot {
            ac_online: Some(true),
            battery_present: false,
            battery_discharging: None,
        },
        sched_ext_available: true,
        vm_knobs: std::collections::BTreeMap::new(),
        inventory_hash: format!("fixture-{}", case.situation),
    };
    let mut active_config = ActiveConfigSnapshot::default();
    seed_task_active_config(&mut active_config, &observation.active_tasks);

    if case.cpu_power_evidence {
        observation.objective_signals.cpu_power_limited = Some(true);
        observation.objective_signals.cpu_power_limited_cpu = Some(0);
        observation.objective_signals.signal_quality.cpu_power = ObjectiveSignalQuality::Derived;
        observation.objective_signals.thermal_degraded = Some(false);
        observation.objective_signals.thermal_throttle_count = Some(0);
        observation.objective_signals.signal_quality.thermal = ObjectiveSignalQuality::Direct;
        inventory
            .cpu_policies
            .push(crate::system_inventory::CpuPolicyInventory {
                policy: "policy0".to_owned(),
                path: std::path::PathBuf::from("/fake/sys/devices/system/cpu/cpufreq/policy0"),
                scaling_governor: Some("powersave".to_owned()),
                available_governors: Some("powersave performance".to_owned()),
                energy_performance_preference: Some("balance_power".to_owned()),
                energy_performance_available_preferences: Some(
                    "balance_power performance".to_owned(),
                ),
                related_cpus: Some("0 1".to_owned()),
            });
        active_config.cpu_power.policies.push(
            crate::autotune::observation::CpuPolicyRuntimeState {
                policy: "policy0".to_owned(),
                scaling_governor: Some("powersave".to_owned()),
                energy_performance_preference: Some("balance_power".to_owned()),
                related_cpus: Some("0 1".to_owned()),
            },
        );
    }

    if case.gpu_power_evidence {
        observation.objective_signals.gpu_power_limited = Some(true);
        observation.objective_signals.gpu_busy_percent = Some(96);
        observation.objective_signals.gpu_clock_mhz = Some(250);
        observation.objective_signals.gpu_temp_millidegrees = Some(70_000);
        observation.objective_signals.gpu_active_render_node = Some("renderD128".to_owned());
        observation.objective_signals.signal_quality.gpu_power = ObjectiveSignalQuality::Direct;
        observation
            .objective_signals
            .signal_quality
            .gpu_active_render_node = ObjectiveSignalQuality::Direct;
        inventory
            .drm_devices
            .push(crate::system_inventory::DrmDeviceInventory {
                name: "card0".to_owned(),
                path: std::path::PathBuf::from("/fake/sys/class/drm/card0"),
                render_node: Some("renderD128".to_owned()),
                pci_id: Some("1002:744c".to_owned()),
                vendor: Some("amd".to_owned()),
                hwmon_paths: Vec::new(),
            });
        active_config
            .gpu_power
            .devices
            .push(crate::autotune::observation::GpuPowerRuntimeState {
                device: "card0".to_owned(),
                power_dpm_force_performance_level: Some("auto".to_owned()),
                pp_power_profile_mode: Some("BOOTUP_DEFAULT".to_owned()),
            });
    }

    if case.irq_evidence {
        observation.objective_signals.irq_overlap_count = Some(2);
        observation.objective_signals.irq_worst_overlap_ns = Some(4_000_000);
        observation.objective_signals.irq_hot_irq = Some(146);
        observation.objective_signals.irq_hot_cpu = Some(2);
        observation.objective_signals.signal_quality.irq_overlap = ObjectiveSignalQuality::Direct;
        active_config.irq.per_irq.insert(146, "4".to_owned());
        inventory.irq_lines.push(crate::irq_inspect::IrqLine {
            irq: "146".to_owned(),
            counts_by_cpu: vec![10_000, 2, 20_000, 30_000],
            total: 60_002,
            kind: "PCI-MSI".to_owned(),
            name: "amdgpu".to_owned(),
            raw: "146: 10000 2 20000 30000 PCI-MSI amdgpu".to_owned(),
        });
    }

    if case.vm_evidence {
        observation.objective_signals.swap_activity_events = Some(3);
        observation.objective_signals.signal_quality.swap_activity =
            ObjectiveSignalQuality::Approximate;
        observation.objective_signals.dirty_writeback_events = Some(5);
        observation.objective_signals.signal_quality.dirty_writeback =
            ObjectiveSignalQuality::Direct;
        observation
            .objective_signals
            .memory_pressure_some_avg10_percent = Some(12.5);
        observation.objective_signals.signal_quality.memory_pressure =
            ObjectiveSignalQuality::Direct;
        inventory
            .vm_knobs
            .insert("proc/sys/vm/swappiness".to_owned(), "60".to_owned());
        active_config
            .vm
            .knobs
            .insert("proc/sys/vm/swappiness".to_owned(), "60".to_owned());
    }

    if let Some(value) = case.hardware_signals.clone() {
        let mut signals: ObjectiveSignals = serde_json::from_value(value)
            .unwrap_or_else(|err| panic!("invalid hardware_signals fixture: {err}"));
        if signals.irq_overlap_count.is_some() || signals.irq_worst_overlap_ns.is_some() {
            signals.signal_quality.irq_overlap = ObjectiveSignalQuality::Direct;
        }
        if signals.block_io_overlap_count.is_some() || signals.block_io_worst_latency_ns.is_some() {
            signals.signal_quality.block_io_overlap = ObjectiveSignalQuality::Direct;
        }
        if signals.thermal_degraded.is_some() || signals.thermal_throttle_count.is_some() {
            signals.signal_quality.thermal = ObjectiveSignalQuality::Direct;
        }
        if signals.cpu_power_limited.is_some() || signals.cpu_power_limited_cpu.is_some() {
            signals.signal_quality.cpu_power = ObjectiveSignalQuality::Derived;
        }
        if signals.gpu_power_limited.is_some()
            || signals.gpu_busy_percent.is_some()
            || signals.gpu_clock_mhz.is_some()
            || signals.gpu_temp_millidegrees.is_some()
        {
            signals.signal_quality.gpu_power = ObjectiveSignalQuality::Direct;
        }
        if signals.gpu_active_render_node.is_some() {
            signals.signal_quality.gpu_active_render_node = ObjectiveSignalQuality::Direct;
        }
        if signals.memory_pressure_some_avg10_percent.is_some() {
            signals.signal_quality.memory_pressure = ObjectiveSignalQuality::Direct;
        }
        if signals.swap_activity_events.is_some() {
            signals.signal_quality.swap_activity = ObjectiveSignalQuality::Approximate;
        }
        if signals.dirty_writeback_events.is_some() {
            signals.signal_quality.dirty_writeback = ObjectiveSignalQuality::Direct;
        }

        if let Some(irq) = signals.irq_hot_irq {
            active_config.irq.per_irq.insert(irq, "4".to_owned());
            inventory.irq_lines.push(crate::irq_inspect::IrqLine {
                irq: irq.to_string(),
                counts_by_cpu: vec![10_000, 2, 20_000, 30_000],
                total: 60_002,
                kind: "PCI-MSI".to_owned(),
                name: "amdgpu".to_owned(),
                raw: format!("{irq}: 10000 2 20000 30000 PCI-MSI amdgpu"),
            });
        }

        if signals.gpu_power_limited == Some(true)
            || signals.gpu_busy_percent.is_some()
            || signals.gpu_active_render_node.is_some()
        {
            let render_node = signals
                .gpu_active_render_node
                .clone()
                .unwrap_or_else(|| "renderD128".to_owned());
            inventory
                .drm_devices
                .push(crate::system_inventory::DrmDeviceInventory {
                    name: "card0".to_owned(),
                    path: std::path::PathBuf::from("/fake/sys/class/drm/card0"),
                    render_node: Some(render_node),
                    pci_id: Some("1002:744c".to_owned()),
                    vendor: Some("amd".to_owned()),
                    hwmon_paths: Vec::new(),
                });
            active_config.gpu_power.devices.push(
                crate::autotune::observation::GpuPowerRuntimeState {
                    device: "card0".to_owned(),
                    power_dpm_force_performance_level: Some("auto".to_owned()),
                    pp_power_profile_mode: Some("BOOTUP_DEFAULT".to_owned()),
                },
            );
        }

        if signals
            .memory_pressure_some_avg10_percent
            .is_some_and(|value| value > 0.0)
            || signals.swap_activity_events.is_some_and(|value| value > 0)
            || signals
                .dirty_writeback_events
                .is_some_and(|value| value > 0)
        {
            inventory
                .vm_knobs
                .insert("proc/sys/vm/swappiness".to_owned(), "60".to_owned());
            active_config
                .vm
                .knobs
                .insert("proc/sys/vm/swappiness".to_owned(), "60".to_owned());
        }

        observation.objective_signals = signals;
    }

    let system_context = SystemContextSnapshot {
        capabilities: observation.capabilities.clone(),
        health: observation.system_health.clone(),
        inventory,
        active_config: active_config.clone(),
        sampled_at_unix_nanos: observation.now_unix_nanos,
    };

    observation.active_config_snapshot = Some(active_config);
    observation.system_context = Some(system_context);
    observation
}

pub(super) fn seed_task_active_config(
    active_config: &mut ActiveConfigSnapshot,
    active_tasks: &[ActiveTaskSnapshot],
) {
    for task in active_tasks {
        active_config
            .affinity
            .per_tid
            .entry(task.tid.as_u32())
            .or_insert_with(|| "0-3".to_owned());
        active_config
            .nice
            .per_tid
            .entry(task.tid.as_u32())
            .or_insert(0);
        active_config
            .ionice
            .per_tid
            .entry(task.tid.as_u32())
            .or_insert_with(|| "best-effort:4".to_owned());
        active_config
            .uclamp
            .per_tid
            .entry(task.tid.as_u32())
            .or_insert(UclampValues {
                sched_util_min: Some(0),
                sched_util_max: Some(1024),
            });
        if let Some(cgroup_path) = &task.cgroup_path {
            active_config
                .cgroup
                .per_tid
                .entry(task.tid.as_u32())
                .or_insert_with(|| cgroup_path.clone());
        }
    }
}

pub(super) fn active_nice_snapshot_for_tasks(
    active_tasks: &[ActiveTaskSnapshot],
    nice: i32,
) -> ActiveConfigSnapshot {
    let mut snapshot = ActiveConfigSnapshot::default();
    for task in active_tasks {
        snapshot.nice.per_tid.insert(task.tid.as_u32(), nice);
    }
    snapshot
}

pub(super) fn fixture_tasks(
    situation: SituationKind,
    focus_kind: FocusGroupKind,
) -> Vec<ActiveTaskSnapshot> {
    match (situation, focus_kind) {
        (
            SituationKind::GameFocused
            | SituationKind::GameCpuSchedulerPressure
            | SituationKind::GameGpuBound,
            _,
        ) => vec![
            fixture_task(1234, "game-main", TaskClass::Game),
            fixture_task(1235, "game-render", TaskClass::GameRenderThread),
            fixture_task(1236, "game-worker", TaskClass::GameWorkerThread),
        ],
        (_, FocusGroupKind::Browser) => vec![
            fixture_task(1234, "browser-main", TaskClass::BrowserForeground),
            fixture_task(1235, "browser-renderer", TaskClass::BrowserRenderer),
            fixture_task(1236, "browser-gpu", TaskClass::BrowserGpu),
        ],
        (_, FocusGroupKind::Compile) => vec![
            fixture_task(1234, "rustc", TaskClass::Compiler),
            fixture_task(1235, "ld.lld", TaskClass::Linker),
        ],
        (SituationKind::CompositorPressure, _) => {
            vec![fixture_task(1234, "compositor-helper", TaskClass::Helper)]
        }
        (_, FocusGroupKind::Media) => {
            vec![fixture_task(1234, "media-player", TaskClass::Media)]
        }
        (_, FocusGroupKind::Recording) => {
            vec![fixture_task(1234, "recorder", TaskClass::Recorder)]
        }
        (_, FocusGroupKind::VirtualMachine) => {
            vec![fixture_task(1234, "qemu", TaskClass::VirtualMachine)]
        }
        _ => vec![fixture_task(1234, "desktop-helper", TaskClass::Helper)],
    }
}

pub(super) fn fixture_task(tid: u32, comm: &str, class: TaskClass) -> ActiveTaskSnapshot {
    let process_starttime_ticks = Some(10);
    let task_starttime_ticks = if tid == 1234 {
        process_starttime_ticks
    } else {
        Some(u64::from(tid))
    };

    ActiveTaskSnapshot {
        tid: tid.into(),
        process_pid: (1234).into(),
        comm: comm.to_owned(),
        class,
        process_starttime_ticks,
        task_starttime_ticks,
        cgroup_path: Some("/user.slice/fixture.scope".to_owned()),
    }
}

pub(super) fn fixture_profiles(case: &PlannerGoldenCase) -> Vec<Profile> {
    if !case
        .policy
        .enabled_action_families
        .iter()
        .any(|family| family == "cpu_affinity_profile")
    {
        return Vec::new();
    }

    let profile_name =
        if parse_fixture_situation(&case.situation) == SituationKind::CompositorPressure {
            "fixture-game-compositor-separate"
        } else {
            "fixture-game-helper"
        };

    vec![Profile {
        name: profile_name.to_owned(),
        rules: vec![ProfileRule {
            affinity: Some(CpuMask::parse("0").unwrap()),
            nice: None,
            ionice: None,
            match_class: vec![
                TaskClass::Game,
                TaskClass::GameHelper,
                TaskClass::GameRenderThread,
                TaskClass::GameWorkerThread,
                TaskClass::Helper,
            ],
            match_comm: Vec::new(),
        }],
    }]
}

pub(super) fn first_fixture_action_kind(case: &PlannerGoldenCase) -> &str {
    case.expected_evaluations
        .first()
        .map(|evaluation| evaluation.action_kind.as_str())
        .or_else(|| {
            case.policy
                .enabled_action_families
                .first()
                .map(String::as_str)
        })
        .unwrap_or("cpu_affinity_profile")
}

pub(super) fn state_candidate_for_action_kind(
    action_kind: &str,
    profiles: &[Profile],
) -> CandidateAction {
    if action_kind == "cpu_affinity_profile" {
        let profile = profiles.first().cloned().unwrap_or_else(|| {
            if let CandidateAction::CpuAffinityProfile { plan } =
                cpu_affinity_candidate("fixture-game-helper")
            {
                plan.profile
            } else {
                panic!("expected cpu affinity candidate")
            }
        });
        return CandidateAction::cpu_affinity_profile(profile, 1234);
    }

    candidate_for_action_kind(action_kind)
}

pub(super) fn parse_fixture_situation(value: &str) -> SituationKind {
    crate::autotune::workload_policy::parse_situation_kind(value).unwrap()
}

pub(super) fn parse_fixture_focus_kind(value: &str) -> FocusGroupKind {
    match value {
        "Game" => FocusGroupKind::Game,
        "Desktop" => FocusGroupKind::Desktop,
        "Browser" => FocusGroupKind::Browser,
        "Compile" => FocusGroupKind::Compile,
        "Recording" => FocusGroupKind::Recording,
        "Media" => FocusGroupKind::Media,
        "VirtualMachine" => FocusGroupKind::VirtualMachine,
        other => panic!("unsupported focus kind {other}"),
    }
}
