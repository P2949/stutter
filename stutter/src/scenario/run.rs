use std::{path::PathBuf, time::Duration};

use anyhow::Result;

use super::{identity::*, io::*, model::*};
use crate::config::model::MonitorConfig;

pub struct ScenarioRunInput {
    pub name: String,
    pub role: ScenarioRole,
    pub dry_run: bool,
    pub out_dir: Option<PathBuf>,
    pub mangohud_log_override: Option<PathBuf>,
}

pub struct PreparedScenarioRun {
    pub config: MonitorConfig,
    pub record: ScenarioRunRecord,
    pub dry_run: bool,
    pub dry_run_text: String,
    pub start_text: String,
}

pub fn prepare_scenario_run(input: ScenarioRunInput) -> Result<PreparedScenarioRun> {
    let scenario = load_scenario(&input.name)?;
    let role = input.role;

    let state_dir = default_scenario_state_dir(&input.name)?;
    let timestamp = crate::audit::unix_nanos_now();
    let run_id = format!("run-{}", timestamp);

    let out_dir = if let Some(dir) = input.out_dir {
        dir
    } else {
        state_dir.join(role.as_str()).join(&run_id)
    };

    let run_name = format!("scenario-{}-{}", role.as_str(), scenario.name);

    // Build Config
    let preset = scenario.preset.parse::<crate::presets::Preset>()?;
    let preset_defaults = preset.defaults();

    fn merge_opt_bool(preset_val: Option<bool>, scenario_val: Option<bool>) -> bool {
        scenario_val.or(preset_val).unwrap_or(false)
    }

    let hwmon = merge_opt_bool(preset_defaults.hwmon, scenario.hwmon);
    let faults = merge_opt_bool(preset_defaults.faults, scenario.faults);
    let stat_wait = merge_opt_bool(preset_defaults.stat_wait, scenario.stat_wait);
    let block_io = merge_opt_bool(preset_defaults.block_io, scenario.block_io);
    let runtime_slices = preset_defaults.runtime_slices.unwrap_or(false);
    let kms_timing = preset_defaults.kms_timing.unwrap_or(false);
    let drm_fence_latency = preset_defaults.drm_fence_latency.unwrap_or(false);
    let wayland_presentation = preset_defaults.wayland_presentation.unwrap_or(false);
    let gpu_engine_sampling = preset_defaults.gpu_engine_sampling.unwrap_or(false);
    let display_topology = preset_defaults.display_topology.unwrap_or(false);
    let foreground_window = preset_defaults.foreground_window.unwrap_or(false);
    let irq_latency = scenario.irq_latency;

    let cpu_freq_config = scenario
        .cpu_freq
        .or(preset_defaults.cpu_freq)
        .unwrap_or(false);
    let cpu_freq = (cpu_freq_config || true) && scenario.cpu_freq.unwrap_or(true);

    let workload_label = scenario
        .watch_process
        .clone()
        .or_else(|| scenario.tree_pid.map(|pid| format!("tree-pid-{pid}")))
        .or_else(|| {
            (!scenario.pid.is_empty()).then(|| format!("pid-count-{}", scenario.pid.len()))
        });
    let scenario_hash = scenario_identity_hash(
        Some(&scenario.name),
        workload_label.as_deref(),
        Some(&scenario.name),
    );

    let config = MonitorConfig {
        target: crate::config::model::TargetConfig {
            target_pids: scenario.pid.clone(),
            tree_pids: scenario.tree_pid.map(|p| vec![p]).unwrap_or_default(),
            include_comm: scenario.include_comm.clone(),
            exclude_comm: scenario.exclude_comm.clone(),
            watch_process: scenario.watch_process.clone(),
            persistent: scenario.persistent,
            max_tasks: 1024,
            ..Default::default()
        },
        timing: crate::config::model::TimingConfig {
            summary_period_ms: scenario.summary_ms.unwrap_or(1000),
            epoch_period_ms: scenario.summary_ms,
            spike_threshold_ns: scenario.spike_us.unwrap_or(1000) * 1000,
            max_duration: Some(Duration::from_secs(scenario.duration)),
        },
        probes: crate::config::model::ProbeConfig {
            irq_latency,
            irqs: scenario.irqs.clone(),
            hwmon,
            cpu_freq,
            faults,
            block_io,
            stat_wait,
            runtime_slices,
            kms_timing,
            drm_fence_latency,
            wayland_presentation,
            gpu_engine_sampling,
            display_topology,
            ..Default::default()
        },
        focus: crate::config::model::FocusConfig {
            foreground_window,
            ..Default::default()
        },
        recording: crate::config::model::RecordingConfig {
            run_name: Some(run_name.clone()),
            scenario_name: Some(scenario.name.clone()),
            scenario_hash,
            workload_label,
            route_label: Some(scenario.name.clone()),
            output_dir: Some(out_dir.clone()),
            ..Default::default()
        },
        mangohud: crate::config::model::MangoHudConfig {
            log: input
                .mangohud_log_override
                .or(scenario.mangohud_log.clone()),
            ..Default::default()
        },
        watch: crate::config::model::WatchConfig {
            poll_ms: 2000,
            ..Default::default()
        },
        ..Default::default()
    };

    let dry_run_text = format!(
        "scenario: {}\n\
         role: {}\n\
         duration: {}s\n\
         watch_process: {:?}\n\
         tree_pid: {:?}\n\
         pid: {:?}\n\
         preset: {}\n\
         output: {}\n\
         mangohud_log: {:?}\n\
         expected_classes: {:?}\n\
         notes: {}\n\
         effective collectors:\n\
           hwmon: {}\n\
           cpu_freq: {}\n\
           faults: {}\n\
           stat_wait: {}\n\
           block_io: {}\n\
           irq_latency: {} (irqs: {:?})\n\
         dry run: no recording started\n",
        scenario.name,
        role.as_str(),
        scenario.duration,
        scenario.watch_process,
        scenario.tree_pid,
        scenario.pid,
        scenario.preset,
        out_dir.display(),
        config.mangohud.log,
        scenario.expected_classes,
        scenario.notes.as_deref().unwrap_or(""),
        config.probes.hwmon,
        config.probes.cpu_freq,
        config.probes.faults,
        config.probes.stat_wait,
        config.probes.block_io,
        config.probes.irq_latency,
        config.probes.irqs,
    );

    let start_text = format!(
        "scenario: {}\n\
         role: {}\n\
         notes: {}\n\
         duration: {}s\n\
         output: {}\n\
         Start the route now; recording will follow watch_process {}.\n",
        scenario.name,
        role.as_str(),
        scenario.notes.as_deref().unwrap_or(""),
        scenario.duration,
        out_dir.display(),
        scenario.watch_process.as_deref().unwrap_or("None"),
    );

    let record = ScenarioRunRecord {
        role,
        run_dir: out_dir,
        run_name,
        unix_nanos: timestamp,
        duration: scenario.duration,
        notes: scenario.notes,
    };

    Ok(PreparedScenarioRun {
        config,
        record,
        dry_run: input.dry_run,
        dry_run_text,
        start_text,
    })
}
