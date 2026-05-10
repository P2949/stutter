use anyhow::Context;

use crate::{
    advisor, audit, cli, commands::input, community_rules, doctor, irq_inspect, probe_catalog,
    process_tree, profiles, tune, watch,
};

pub async fn run_apply_profile_command(
    input: input::ApplyProfileCommandInput,
) -> anyhow::Result<()> {
    watch::apply_profile_command(watch::ApplyProfileCommandInput {
        tree_pid: input.tree_pid,
        profile_path: input.profile,
        force: input.force,
        dry_run: input.dry_run,
        allow_medium_risk: input.allow_medium_risk,
        watch: input.watch,
        keep_applied: input.keep_applied,
        refresh_ms: input.refresh_ms,
        enforce: input.enforce,
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
        out_dir: input.out_dir,
        mangohud_log: input.mangohud_log,
        enforce: input.enforce,
        hwmon: input.hwmon,
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
    probe_catalog::probes_command(input.json)
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
