use crate::{autotune, commands::input};

pub fn run_generate_profiles_command(
    input: input::AutotuneGenerateProfilesCommandInput,
) -> anyhow::Result<()> {
    crate::autotune::generate_profiles::generate_profiles_command(
        crate::autotune::generate_profiles::GenerateProfilesCommandInput {
            watch_process: input.watch_process,
            out: input.out,
            allow_cpus: input.allow_cpus,
            deny_cpus: input.deny_cpus,
            min_render_cpus: input.min_render_cpus,
            min_game_cpus: input.min_game_cpus,
            min_compositor_cpus: input.min_compositor_cpus,
            min_background_cpus: input.min_background_cpus,
        },
    )
}

pub async fn run_autotune_command(input: input::AutotuneCommandInput) -> anyhow::Result<()> {
    autotune::commands::live::autotune_command(input.input).await
}

pub fn run_apply_candidate_command(
    input: input::AutotuneApplyCandidateCommandInput,
) -> anyhow::Result<()> {
    let plan =
        autotune::candidate::apply_candidate_plan_file(&input.candidate_json, input.dry_run)?;
    if input.dry_run {
        println!(
            "candidate_plan_dry_run candidate={} action_kind={} safety={:?} objective={:?}",
            plan.candidate.candidate_name,
            plan.candidate.action_kind,
            plan.descriptor.safety_class,
            plan.objective
        );
    }
    Ok(())
}

pub fn run_status_command(input: input::AutotuneStatusCommandInput) -> anyhow::Result<()> {
    autotune::status::autotune_status_command(autotune::status::AutotuneStatusCommandInput {
        json: input.json,
        history_path: None,
    })
}

pub fn run_replay_history_command(
    input: input::AutotuneReplayHistoryCommandInput,
) -> anyhow::Result<()> {
    autotune::history_replay::autotune_replay_history_command(
        autotune::history_replay::AutotuneReplayHistoryCommandInput {
            history_path: input.history,
        },
    )
}

pub fn run_restore_command(input: input::AutotuneRestoreCommandInput) -> anyhow::Result<()> {
    autotune::emergency_restore::autotune_restore_command(
        autotune::emergency_restore::AutotuneRestoreCommandInput {
            journal_path: input.journal,
            audit_path: input.audit,
            history_path: input.history,
            dry_run: input.dry_run,
        },
    )
}

pub fn run_replay_command(input: input::AutotuneReplayCommandInput) -> anyhow::Result<()> {
    autotune::replay::replay_command(autotune::replay::AutotuneReplayInput {
        run_dir: input.run,
        config_path: input.config,
    })
}
