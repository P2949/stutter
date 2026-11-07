//! Controlled-real and unattended real-game evidence (package D): the frozen manifest, the paired
//! statistics and the preregistered classification, and the honest inventory of what can run.
mod inventory;
mod manifest;
mod outcome;
#[cfg(test)]
mod tests;

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};
use clap::{Args, Subcommand};
pub use manifest::Manifest;

#[derive(Args, Clone, Debug, Eq, PartialEq)]
pub struct RealWorkloadArgs {
    #[command(subcommand)]
    pub command: RealWorkloadCommand,
}

#[derive(Subcommand, Clone, Debug, Eq, PartialEq)]
pub enum RealWorkloadCommand {
    /// Check a frozen manifest and print its digest.
    Validate { manifest: PathBuf },
    /// Classify measured pairs by a frozen manifest.
    Evaluate {
        manifest: PathBuf,
        /// JSON: {"facts": {...}, "pairs": [...]}.
        results: PathBuf,
    },
    /// Record what real-workload material this host has and its disposition.
    Inventory {
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        steamapps: Option<PathBuf>,
        /// One title per line: games with a repeatable unattended benchmark or replay.
        #[arg(long)]
        allowlist: Option<PathBuf>,
    },
}

#[derive(serde::Deserialize)]
struct Results {
    facts: outcome::RunFacts,
    pairs: Vec<outcome::PairResult>,
}

fn read_manifest(path: &Path) -> anyhow::Result<Manifest> {
    let manifest: Manifest = serde_json::from_slice(&fs::read(path)?)
        .with_context(|| format!("{} is not a manifest", path.display()))?;
    manifest.validate()?;
    Ok(manifest)
}

pub fn run(args: &RealWorkloadArgs) -> anyhow::Result<()> {
    match &args.command {
        RealWorkloadCommand::Validate { manifest } => {
            println!("{}", read_manifest(manifest)?.frozen_digest()?);
            Ok(())
        }
        RealWorkloadCommand::Evaluate { manifest, results } => {
            let manifest = read_manifest(manifest)?;
            let results: Results = serde_json::from_slice(&fs::read(results)?)?;
            let verdict = outcome::classify(&manifest, &results.facts, &results.pairs);
            println!("{}", serde_json::to_string_pretty(&verdict)?);
            Ok(())
        }
        RealWorkloadCommand::Inventory {
            out,
            steamapps,
            allowlist,
        } => {
            let steamapps = steamapps.clone().unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                    .join(".local/share/Steam/steamapps")
            });
            let allowed: Vec<String> = match allowlist {
                Some(path) => fs::read_to_string(path)
                    .with_context(|| format!("cannot read {}", path.display()))?
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty() && !line.starts_with('#'))
                    .map(str::to_owned)
                    .collect(),
                None => Vec::new(),
            };
            let inventory = inventory::collect(&steamapps, &allowed);
            if inventory.steam_titles.is_empty() && !steamapps.exists() {
                bail!("{} does not exist; pass --steamapps", steamapps.display());
            }
            fs::write(out, serde_json::to_string_pretty(&inventory)?)?;
            println!("{}", inventory.real_game_disposition);
            Ok(())
        }
    }
}
