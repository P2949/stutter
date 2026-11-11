//! Owned fixed-work placement study (E2-E5): a frozen manifest, a supervised counterbalanced runner
//! and a deterministic analysis, for candidates that only ever apply to processes this study starts.
//!
//! Nothing here reaches the privileged worker or the daemon registry. Candidates are a closed enum
//! (child CPU affinity, or the weight of a transient user scope); an unknown name, a CPU outside
//! this process's effective set or a no-op candidate is refused before anything is spawned.
use std::path::PathBuf;

use anyhow::Context;
use clap::Subcommand;

mod analyze;
mod diagnostics;
mod host;
mod journal;
mod manifest;
mod run;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_diag;
#[cfg(test)]
mod tests_host;
#[cfg(test)]
mod tests_journal;
#[cfg(test)]
mod tests_workspace;
mod workspace;

#[derive(Subcommand, Clone, Debug, Eq, PartialEq)]
pub enum Command {
    /// Validate a manifest against this host and print its frozen digest.
    Validate {
        #[arg(long)]
        manifest: PathBuf,
    },
    /// Run every unfinished slot (resumable; a recorded slot is never repeated).
    Run {
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Analyse the recorded results into effect intervals and a held-out ranking of legal candidates.
    Analyze {
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        out: PathBuf,
        /// New file for the analysis JSON.
        #[arg(long)]
        report: PathBuf,
    },
}

pub fn execute(command: Command) -> anyhow::Result<()> {
    match command {
        Command::Validate { manifest } => {
            let loaded = manifest::load(&manifest)?;
            loaded.validate(&manifest::effective_cpus_of_self()?)?;
            println!("{}", loaded.digest());
            Ok(())
        }
        Command::Run { manifest, out } => run::run(&manifest::load(&manifest)?, &out),
        Command::Analyze {
            manifest,
            out,
            report,
        } => {
            let loaded = manifest::load(&manifest)?;
            let value = analyze::analyze(&loaded, &out).context("analyse the recorded results")?;
            let file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&report)
                .context("the report file must be new")?;
            serde_json::to_writer_pretty(file, &value)?;
            Ok(())
        }
    }
}
