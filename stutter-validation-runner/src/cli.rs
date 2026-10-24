use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "stutter-validation-runner",
    about = "Validation-only privileged runner"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Serve typed validation actions on a local Unix socket.
    Serve {
        #[arg(long)]
        authorization: PathBuf,
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long, default_value = "/run/stutter-validation/runner.sock")]
        socket: PathBuf,
    },
    /// Resume the controller of every campaign with a reboot in flight.
    Resume {
        #[arg(long)]
        all: bool,
        #[arg(long, default_value = "/var/lib/stutter-validation")]
        state_root: PathBuf,
        /// Wait up to this many seconds for the runner's socket before resuming anything.
        #[arg(long, default_value_t = 0)]
        wait_for_runner: u32,
        #[arg(long, default_value = "/run/stutter-validation/runner.sock")]
        socket: PathBuf,
    },
    /// One-time install from a staged authorization (run via `sudo -n` by `bootstrap`).
    Install {
        #[arg(long)]
        staged: PathBuf,
        #[arg(long)]
        repository: PathBuf,
        /// Install into this directory instead of / (tests, dry runs); never activates services.
        #[arg(long)]
        root: Option<PathBuf>,
    },
    /// Validate an authorization file and print a summary.
    Check {
        #[arg(long)]
        authorization: PathBuf,
    },
}
