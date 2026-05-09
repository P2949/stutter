use std::path::PathBuf;

use anyhow::Context;

use crate::{
    advisor, audit, cli, community_rules, doctor, irq_inspect, probe_catalog, process_tree,
    profiles, tune, watch,
};

#[allow(clippy::too_many_arguments)]
pub async fn run_apply_profile_command(
    tree_pid: u32,
    profile: PathBuf,
    force: bool,
    dry_run: bool,
    allow_medium_risk: bool,
    watch_apply: bool,
    keep_applied: bool,
    refresh_ms: u64,
    enforce: bool,
) -> anyhow::Result<()> {
    watch::apply_profile_command(watch::ApplyProfileCommandInput {
        tree_pid,
        profile_path: profile,
        force,
        dry_run,
        allow_medium_risk,
        watch: watch_apply,
        keep_applied,
        refresh_ms,
        enforce,
    })
    .await
}

pub fn run_inspect_tree_command(tree_pid: u32) -> anyhow::Result<()> {
    let rendered = process_tree::render_tree(tree_pid)?;
    print!("{rendered}");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn run_tune_command(
    tree_pid: u32,
    profiles: PathBuf,
    epoch_seconds: u64,
    warmup_seconds: u64,
    runs: u32,
    keep_best: bool,
    baseline_profile: Option<String>,
    out_dir: Option<PathBuf>,
    mangohud_log: Option<PathBuf>,
    enforce: bool,
    hwmon: bool,
) -> anyhow::Result<()> {
    tune::tune_command(tune::TuneCommandInput {
        tree_pid,
        profiles_path: profiles,
        epoch_seconds,
        warmup_seconds,
        runs,
        keep_best,
        baseline_profile,
        out_dir,
        mangohud_log,
        enforce,
        hwmon,
    })
    .await
}

pub fn run_audit_command(path: Option<PathBuf>, tail: usize, json: bool) -> anyhow::Result<()> {
    audit::audit_command(audit::AuditCommandInput { path, tail, json })
}

#[allow(clippy::too_many_arguments)]
pub async fn run_advisor_command(
    run: Option<PathBuf>,
    profiles: Option<PathBuf>,
    json: bool,
    watch_runs: bool,
    runs_dir: Option<PathBuf>,
    poll_seconds: u64,
    once: bool,
) -> anyhow::Result<()> {
    advisor::advisor_command(advisor::AdvisorCommandInput {
        run,
        profiles,
        json,
        watch_runs,
        runs_dir,
        poll_seconds,
        once,
    })
    .await
}

pub fn run_doctor_command(input: doctor::DoctorInput) -> anyhow::Result<()> {
    doctor::doctor_command(input)
}

pub fn run_probes_command(json: bool) -> anyhow::Result<()> {
    probe_catalog::probes_command(json)
}

pub fn run_profile_template_command(topology: bool) -> anyhow::Result<()> {
    if topology {
        print!("{}", profiles::generate_topology_template());
        Ok(())
    } else {
        anyhow::bail!("profile-template requires --topology");
    }
}

pub fn run_inspect_irqs_command(json: bool, filter: Vec<String>, top: usize) -> anyhow::Result<()> {
    irq_inspect::run_inspect_irqs(json, &filter, top)
}

pub fn run_completions_command(shell: clap_complete::Shell) -> anyhow::Result<()> {
    let mut cmd = cli::command();
    clap_complete::generate(shell, &mut cmd, "stutter", &mut std::io::stdout());
    Ok(())
}

pub fn run_man_command(output: Option<PathBuf>) -> anyhow::Result<()> {
    render_man_page(output.as_deref())
}

pub fn run_rules_command(command: cli::RulesCommand) -> anyhow::Result<()> {
    community_rules::rules_command(command)
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
