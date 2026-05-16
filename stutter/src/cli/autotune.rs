use std::path::PathBuf;

use clap::{Args, Subcommand};
use serde::Serialize;

#[derive(Args, Clone, Debug, Serialize)]
pub struct AutotuneArgs {
    #[command(subcommand)]
    pub command: AutotuneCommand,
}

#[derive(Subcommand, Clone, Debug, Serialize)]
pub enum AutotuneCommand {
    ApplyCandidate(AutotuneApplyCandidateArgs),
}

#[derive(Args, Clone, Debug, Serialize)]
pub struct AutotuneApplyCandidateArgs {
    pub candidate_plan_file: PathBuf,

    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args, Clone, Debug, Serialize)]
pub struct AutotuneStatusArgs {
    #[arg(long)]
    pub tui: bool,

    #[arg(long)]
    pub json: bool,

    #[arg(long, default_value = "10")]
    pub explain_last: usize,
}
