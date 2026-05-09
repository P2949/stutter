use std::path::PathBuf;

use crate::autotune;

pub fn run_generate_profiles_command(
    watch_process: Option<String>,
    out: PathBuf,
    allow_cpus: Option<String>,
    deny_cpus: Option<String>,
    min_render_cpus: usize,
    min_game_cpus: usize,
    min_compositor_cpus: usize,
    min_background_cpus: usize,
) -> anyhow::Result<()> {
    autotune::generate_profiles::generate_profiles_command(
        autotune::generate_profiles::GenerateProfilesCommandInput {
            watch_process,
            out,
            allow_cpus,
            deny_cpus,
            min_render_cpus,
            min_game_cpus,
            min_compositor_cpus,
            min_background_cpus,
        },
    )
}

pub async fn run_autotune_command(input: autotune::AutotuneCommandInput) -> anyhow::Result<()> {
    autotune::autotune_command(input).await
}

pub fn run_status_command(json: bool) -> anyhow::Result<()> {
    autotune::status::autotune_status_command(autotune::status::AutotuneStatusCommandInput {
        json,
        history_path: None,
    })
}

pub fn run_replay_history_command(history: PathBuf) -> anyhow::Result<()> {
    autotune::history_replay::autotune_replay_history_command(
        autotune::history_replay::AutotuneReplayHistoryCommandInput {
            history_path: history,
        },
    )
}

pub fn run_restore_command(
    journal: Option<PathBuf>,
    audit: Option<PathBuf>,
    history: Option<PathBuf>,
    dry_run: bool,
) -> anyhow::Result<()> {
    autotune::emergency_restore::autotune_restore_command(
        autotune::emergency_restore::AutotuneRestoreCommandInput {
            journal_path: journal,
            audit_path: audit,
            history_path: history,
            dry_run,
        },
    )
}

pub fn run_replay_command(run: PathBuf, config: Option<PathBuf>) -> anyhow::Result<()> {
    autotune::replay::replay_command(autotune::replay::AutotuneReplayInput {
        run_dir: run,
        config_path: config,
    })
}
