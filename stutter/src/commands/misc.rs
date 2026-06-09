use anyhow::Context;
use serde::Serialize;

use crate::{
    advisor, audit, cli, commands::input, community_rules, config_file, display_path_compare,
    doctor, drm_fence_tracepoints, irq_inspect, metadata, probe_catalog, process_tree, profiles,
    tune, watch, wayland_probe,
};

pub fn run_version_command(input: input::VersionCommandInput) -> anyhow::Result<()> {
    println!("stutter {}", metadata::build_version());
    if input.features {
        println!("git_rev: {}", metadata::build_git_rev());
        println!("features: {}", metadata::build_feature_labels().join(", "));
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct ConfigCheckOutput {
    ok: bool,
    user_config_loaded: bool,
    diagnostics: Vec<String>,
    daemon_preset: String,
}

pub fn run_config_check_command(input: input::ConfigCheckCommandInput) -> anyhow::Result<()> {
    let user_config = config_file::load_user_config()?;
    if let Some(config) = user_config.as_ref() {
        crate::config::layer::layer_from_user_file(config)?;
        let _ = config_file::agent_autotune_limits_from_user_config(Some(config))?;
    }

    let daemon_preset = user_config
        .as_ref()
        .and_then(|config| config.daemon_preset.as_deref())
        .unwrap_or("observe-only")
        .parse::<crate::daemon::config::DaemonPreset>()?;
    let diagnostics = user_config
        .as_ref()
        .map(|config| {
            config
                .diagnostics
                .iter()
                .map(|diagnostic| diagnostic.message.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let output = ConfigCheckOutput {
        ok: true,
        user_config_loaded: user_config.is_some(),
        diagnostics,
        daemon_preset: daemon_preset.to_string(),
    };

    if input.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else if output.user_config_loaded {
        println!("config ok; daemon_preset={}", output.daemon_preset);
        for diagnostic in &output.diagnostics {
            println!("warning: {diagnostic}");
        }
    } else {
        println!("config ok; no user config file found; daemon_preset=observe-only");
    }

    Ok(())
}

pub async fn run_apply_profile_command(
    input: input::ApplyProfileCommandInput,
) -> anyhow::Result<()> {
    watch::apply_profile_command(watch::ApplyProfileCommandInput {
        tree_pid: input.tree_pid,
        profile_path: input.profile,
        profile_name: input.profile_name,
        force: input.force,
        dry_run: input.dry_run,
        allow_medium_risk: input.allow_medium_risk,
        watch: input.watch,
        keep_applied: input.keep_applied,
        refresh_ms: input.refresh_ms,
        enforce: input.enforce,
        explain: input.explain,
        json: input.json,
        output: input.output,
        top: input.top,
        highlight_comm: input.highlight_comm,
    })
    .await
}

pub async fn run_profile_plan_command(input: input::ProfilePlanCommandInput) -> anyhow::Result<()> {
    watch::profile_plan_command(watch::ProfilePlanCommandInput {
        tree_pid: input.tree_pid,
        profile_path: input.profile,
        profile_name: input.profile_name,
        json: input.json,
        output: input.output,
        top: input.top,
        highlight_comm: input.highlight_comm,
    })
    .await
}

pub fn run_inspect_tree_command(input: input::InspectTreeCommandInput) -> anyhow::Result<()> {
    let rendered = process_tree::render_tree(input.tree_pid)?;
    print!("{rendered}");
    Ok(())
}

pub async fn run_tune_command(input: input::TuneCommandInput) -> anyhow::Result<()> {
    tune::tune_command(tune::TuneCommandInput {
        tree_pid: input.tree_pid,
        profiles_path: input.profiles,
        epoch_seconds: input.epoch_seconds,
        warmup_seconds: input.warmup_seconds,
        runs: input.runs,
        keep_best: input.keep_best,
        baseline_profile: input.baseline_profile,
        scenario_name: input.scenario_name,
        workload_label: input.workload_label,
        route_label: input.route_label,
        out_dir: input.out_dir,
        mangohud_log: input.mangohud_log,
        enforce: input.enforce,
        hwmon: input.hwmon,
        order_strategy: input.order_strategy.clone(),
    })
    .await
}

pub fn run_audit_command(input: input::AuditCommandInput) -> anyhow::Result<()> {
    audit::audit_command(audit::AuditCommandInput {
        path: input.path,
        tail: input.tail,
        json: input.json,
    })
}

pub async fn run_advisor_command(input: input::AdvisorCommandInput) -> anyhow::Result<()> {
    advisor::advisor_command(advisor::AdvisorCommandInput {
        run: input.run,
        profiles: input.profiles,
        json: input.json,
        watch_runs: input.watch_runs,
        runs_dir: input.runs_dir,
        poll_seconds: input.poll_seconds,
        once: input.once,
    })
    .await
}

pub fn run_doctor_command(input: input::DoctorCommandInput) -> anyhow::Result<()> {
    doctor::doctor_command(input.input)
}

pub fn run_probes_command(input: input::ProbesCommandInput) -> anyhow::Result<()> {
    probe_catalog::probes_command(input.json, input.include_planned)
}

pub fn run_profile_template_command(
    input: input::ProfileTemplateCommandInput,
) -> anyhow::Result<()> {
    if input.topology {
        print!("{}", profiles::generate_topology_template());
        Ok(())
    } else {
        anyhow::bail!("profile-template requires --topology");
    }
}

pub fn run_inspect_irqs_command(input: input::InspectIrqsCommandInput) -> anyhow::Result<()> {
    irq_inspect::run_inspect_irqs(input.json, &input.filter, input.top)
}

pub fn run_inspect_drm_tracepoints_command(
    input: input::InspectDrmTracepointsCommandInput,
) -> anyhow::Result<()> {
    let discovery = input
        .events_root
        .as_deref()
        .map(drm_fence_tracepoints::discover_drm_fence_tracepoints)
        .unwrap_or_else(drm_fence_tracepoints::discover_drm_fence_tracepoints_default);
    if input.json {
        println!("{}", serde_json::to_string_pretty(&discovery)?);
    } else {
        print!("{}", drm_fence_tracepoints::render_text(&discovery));
    }
    Ok(())
}

pub fn run_display_path_compare_command(
    input: input::DisplayPathCompareCommandInput,
) -> anyhow::Result<()> {
    display_path_compare::run_display_path_compare(display_path_compare::DisplayPathCompareInput {
        baseline: input.baseline,
        test: input.test,
        json: input.json,
        strict: input.strict,
        expect: input.expect,
    })
}

pub fn run_wayland_probe_command(input: input::WaylandProbeCommandInput) -> anyhow::Result<()> {
    wayland_probe::run_wayland_probe_command(wayland_probe::WaylandProbeCommandInput {
        duration: input.duration,
        output: input.output,
        fullscreen: input.fullscreen,
        out_dir: input.out_dir,
    })
}

pub fn run_completions_command(input: input::CompletionsCommandInput) -> anyhow::Result<()> {
    let mut cmd = cli::command();
    clap_complete::generate(input.shell, &mut cmd, "stutter", &mut std::io::stdout());
    Ok(())
}

pub fn run_man_command(input: input::ManCommandInput) -> anyhow::Result<()> {
    render_man_page(input.output.as_deref())
}

pub fn run_rules_command(input: input::RulesCommandInput) -> anyhow::Result<()> {
    community_rules::rules_command(input.command)
}

fn render_man_page(output: Option<&std::path::Path>) -> anyhow::Result<()> {
    let cmd = cli::command();
    let man = clap_mangen::Man::new(cmd);

    if let Some(path) = output {
        let mut file = std::fs::File::create(path)
            .with_context(|| format!("failed to create man page {}", path.display()))?;
        man.render(&mut file)
            .with_context(|| format!("failed to render man page to {}", path.display()))?;
    } else {
        let stdout = std::io::stdout();
        let mut handle = stdout.lock();
        man.render(&mut handle)
            .with_context(|| "failed to render man page to stdout")?;
    }

    Ok(())
}
