//! Offline benchmark and diagnostic artifact import (package W13).
//!
//! An imported artifact is *evidence about a run*, never authority: it carries no operation, no
//! command that is ever executed, and `authorizes_mutation` is constant false. Import verifies
//! the tool and version, the exact argv, the build id, the role and phase, the unit, the
//! independent run id, the clock window, the population and loss, the exit status and the SHA-256
//! of every artifact file, and refuses an oversize, truncated or tampered file.
mod diagnostics;
#[cfg(test)]
mod diagnostics_tests;
mod hyperfine;
mod scores;
#[cfg(test)]
mod scores_tests;
#[cfg(test)]
mod tests;

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, bail, ensure};
use clap::{Args, Subcommand};
use serde::{Deserialize, Serialize};

use crate::evidence_bundle::sha256::sha256_hex;

/// Largest artifact file read, in bytes.
pub const MAX_ARTIFACT_BYTES: u64 = 8 * 1024 * 1024;
/// Most samples taken from one artifact.
pub const MAX_SAMPLES: usize = 100_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Baseline,
    Treatment,
    Calibration,
    Diagnostic,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArtifactFile {
    pub path: String,
    pub sha256: String,
}

/// What an external tool run is claimed to have been.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArtifactManifest {
    pub schema_version: u32,
    pub tool: String,
    pub tool_version: String,
    /// The exact argv that ran; recorded, never executed by this importer.
    pub argv: Vec<String>,
    /// Build id or digest of the binary under test.
    pub build_id: String,
    pub role: Role,
    pub phase: String,
    pub unit: String,
    /// A run id independent of every other run in the comparison.
    pub run_id: String,
    pub window_start_unix: u64,
    pub window_end_unix: u64,
    /// Events the tool says it measured, and events it says it lost.
    pub population: u64,
    pub lost: u64,
    pub exit_code: i32,
    pub files: Vec<ArtifactFile>,
}

impl ArtifactManifest {
    /// An imported artifact never authorizes an operation.
    pub const fn authorizes_mutation(&self) -> bool {
        false
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        ensure!(self.schema_version == 1, "unsupported schema_version");
        for (name, value) in [
            ("tool", &self.tool),
            ("tool_version", &self.tool_version),
            ("build_id", &self.build_id),
            ("phase", &self.phase),
            ("unit", &self.unit),
            ("run_id", &self.run_id),
        ] {
            ensure!(!value.trim().is_empty(), "{name} is empty");
        }
        ensure!(!self.argv.is_empty(), "argv is empty");
        ensure!(
            self.window_end_unix >= self.window_start_unix,
            "the clock window ends before it starts"
        );
        ensure!(self.population > 0, "population is zero");
        ensure!(!self.files.is_empty(), "no artifact file is listed");
        for file in &self.files {
            ensure!(
                !file.path.contains("..") && !file.path.starts_with('/'),
                "artifact path {:?} leaves the manifest directory",
                file.path
            );
            ensure!(
                file.sha256.len() == 64 && file.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
                "artifact {} has no valid sha256",
                file.path
            );
        }
        Ok(())
    }
}

/// A verified artifact: the manifest and the bytes of each file, digest-checked.
#[derive(Debug)]
pub struct Verified {
    pub manifest: ArtifactManifest,
    pub files: Vec<(String, Vec<u8>)>,
    /// Lost events as a share of all events, so a lossy capture is visible.
    pub loss_fraction: f64,
    pub clean_exit: bool,
}

pub fn verify(manifest_path: &Path) -> anyhow::Result<Verified> {
    let manifest: ArtifactManifest = serde_json::from_slice(&fs::read(manifest_path)?)
        .with_context(|| format!("{} is not an artifact manifest", manifest_path.display()))?;
    manifest.validate()?;
    let dir = manifest_path.parent().unwrap_or(Path::new("."));
    let mut files = Vec::new();
    for file in &manifest.files {
        let path = dir.join(&file.path);
        let meta = fs::symlink_metadata(&path)
            .with_context(|| format!("artifact {} is missing", file.path))?;
        ensure!(
            meta.file_type().is_file(),
            "artifact {} is not a regular file",
            file.path
        );
        ensure!(
            meta.len() <= MAX_ARTIFACT_BYTES,
            "artifact {} is {} bytes, over the {MAX_ARTIFACT_BYTES} byte limit",
            file.path,
            meta.len()
        );
        let bytes = fs::read(&path)?;
        ensure!(
            sha256_hex(&bytes) == file.sha256,
            "artifact {} does not match its recorded sha256: it was altered or truncated",
            file.path
        );
        files.push((file.path.clone(), bytes));
    }
    let total = manifest.population.saturating_add(manifest.lost) as f64;
    Ok(Verified {
        loss_fraction: manifest.lost as f64 / total,
        clean_exit: manifest.exit_code == 0,
        manifest,
        files,
    })
}

#[derive(Args, Clone, Debug, Eq, PartialEq)]
pub struct BenchmarksArgs {
    #[command(subcommand)]
    pub command: BenchmarksCommand,
}

#[derive(Subcommand, Clone, Debug, Eq, PartialEq)]
pub enum BenchmarksCommand {
    /// Verify an artifact manifest and every file it lists.
    Verify { manifest: PathBuf },
    /// Read a `hyperfine --export-json` file as run samples.
    ImportHyperfine { file: PathBuf },
    /// Read sysbench, stress-ng, benchstat or Phoronix output as typed, non-proof scores.
    ImportScores { tool: ScoreTool, file: PathBuf },
}

#[derive(clap::ValueEnum, Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScoreTool {
    Sysbench,
    StressNg,
    Benchstat,
    Phoronix,
    Iostat,
    Pidstat,
    Numastat,
    Blkparse,
    Tuna,
    Cyclictest,
    BpftraceHist,
    BccHist,
    TraceCmd,
}

fn import_scores(tool: ScoreTool, file: &Path) -> anyhow::Result<String> {
    ensure!(
        fs::metadata(file)?.len() <= MAX_ARTIFACT_BYTES,
        "file is over the size limit"
    );
    let text = String::from_utf8(fs::read(file)?).context("file is not UTF-8 text")?;
    Ok(match tool {
        ScoreTool::Sysbench => serde_json::to_string_pretty(&scores::import_sysbench(&text)?)?,
        ScoreTool::StressNg => serde_json::to_string_pretty(&scores::import_stress_ng(&text)?)?,
        ScoreTool::Benchstat => serde_json::to_string_pretty(&scores::import_benchstat(&text)?)?,
        ScoreTool::Phoronix => serde_json::to_string_pretty(&scores::import_phoronix(&text)?)?,
        ScoreTool::Iostat => serde_json::to_string_pretty(&diagnostics::import_iostat(&text)?)?,
        ScoreTool::Pidstat => serde_json::to_string_pretty(&diagnostics::import_pidstat(&text)?)?,
        ScoreTool::Numastat => serde_json::to_string_pretty(&diagnostics::import_numastat(&text)?)?,
        ScoreTool::Blkparse => serde_json::to_string_pretty(&diagnostics::import_blkparse(&text)?)?,
        ScoreTool::Tuna => serde_json::to_string_pretty(&diagnostics::import_tuna(&text)?)?,
        ScoreTool::Cyclictest => {
            serde_json::to_string_pretty(&diagnostics::import_cyclictest(&text)?)?
        }
        ScoreTool::BpftraceHist => {
            serde_json::to_string_pretty(&diagnostics::import_bpftrace_hist(&text)?)?
        }
        ScoreTool::BccHist => serde_json::to_string_pretty(&diagnostics::import_bcc_hist(&text)?)?,
        ScoreTool::TraceCmd => {
            serde_json::to_string_pretty(&diagnostics::import_trace_cmd(&text)?)?
        }
    })
}

pub fn run(args: &BenchmarksArgs) -> anyhow::Result<()> {
    match &args.command {
        BenchmarksCommand::ImportScores { tool, file } => {
            println!("{}", import_scores(*tool, file)?);
            Ok(())
        }
        BenchmarksCommand::Verify { manifest } => {
            let verified = verify(manifest)?;
            println!(
                "verified {} {} run {}: {} file(s), loss {:.2}%, exit {}",
                verified.manifest.tool,
                verified.manifest.tool_version,
                verified.manifest.run_id,
                verified.files.len(),
                verified.loss_fraction * 100.0,
                verified.manifest.exit_code
            );
            if !verified.clean_exit {
                bail!(
                    "the tool exited with {}: not a complete run",
                    verified.manifest.exit_code
                );
            }
            Ok(())
        }
        BenchmarksCommand::ImportHyperfine { file } => {
            let text = fs::read(file)?;
            ensure!(
                text.len() as u64 <= MAX_ARTIFACT_BYTES,
                "file is over the size limit"
            );
            let runs = hyperfine::import(&text)?;
            println!("{}", serde_json::to_string_pretty(&runs)?);
            Ok(())
        }
    }
}
