use std::path::PathBuf;

use clap::{Args, Subcommand};

#[derive(Args, Clone, Debug, Eq, PartialEq)]
pub struct CampaignArgs {
    #[command(subcommand)]
    pub command: CampaignCommand,
}

#[derive(Subcommand, Clone, Debug, Eq, PartialEq)]
pub enum CampaignCommand {
    /// Create a campaign instance from the current clean `main`.
    Init {
        #[arg(long)]
        manifest: PathBuf,
    },
    /// Detect (and later install) the privileged validation runner.
    Bootstrap {
        #[arg(long)]
        campaign: Option<PathBuf>,
    },
    /// Show campaign state; `--json` is the stable machine-readable form.
    Status {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        campaign: Option<PathBuf>,
    },
    /// Predict the next runnable task without executing anything.
    Next {
        #[arg(long)]
        campaign: Option<PathBuf>,
    },
    /// Run everything immediately runnable until a genuine boundary.
    Continue {
        #[arg(long)]
        campaign: Option<PathBuf>,
        /// Stop after N tasks (bounded testing).
        #[arg(long)]
        max_tasks: Option<usize>,
        /// Resume after a reboot (invoked by the validation resume service).
        #[arg(long)]
        resume: bool,
    },
    /// Rebuild campaign truth from retained receipts instead of cached statuses.
    Verify {
        #[arg(long)]
        campaign: Option<PathBuf>,
        #[arg(long)]
        strict: bool,
    },
    /// Write JSON and Markdown summaries.
    Report {
        #[arg(long)]
        campaign: Option<PathBuf>,
    },
    /// Stop future automation; collected evidence is never deleted.
    Cancel {
        #[arg(long)]
        campaign: Option<PathBuf>,
    },
}
