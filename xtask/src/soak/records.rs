//! Durable soak records: the state that lets a soak resume after a crash or a reboot, and the
//! per-round record every acceptance check reads back.
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};

pub(super) const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    /// The real soak: realistic experiment windows, rounds of minutes.
    Full,
    /// The same event types on short windows, to shake the soak itself out in about an hour.
    Smoke,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SoakState {
    pub schema_version: u32,
    pub id: String,
    pub profile: Profile,
    pub seed: u64,
    pub hours: f64,
    pub stutter_bin: PathBuf,
    pub stutter_sha256: String,
    pub xtask_sha256: String,
    pub source_commit: String,
    /// A trial run on a dirty tree: never publishable evidence.
    #[serde(default)]
    pub allow_dirty: bool,
    pub created_unix_ms: u64,
    /// Set by the first `run`; the soak ends at `started + hours` of wall-clock time.
    pub started_unix_ms: Option<u64>,
    pub cpus: Option<(u32, u32)>,
    pub next_round: u32,
    pub finished: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct EventRecord {
    pub unix_ms: u64,
    pub kind: String,
    pub detail: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct SessionRecord {
    pub start_unix_ms: u64,
    pub end_unix_ms: u64,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub ended_by: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct RoundRecord {
    pub round: u32,
    pub kind: String,
    pub scenario: String,
    pub operations: String,
    pub started_unix_ms: u64,
    pub finished_unix_ms: u64,
    /// `ok`, `failed` or `interrupted` (the soak process itself died mid-round).
    pub outcome: String,
    pub error: Option<String>,
    pub sessions: Vec<SessionRecord>,
    pub events: Vec<EventRecord>,
    /// Findings of the quiescence checks and the sampler; empty for a clean round.
    pub violations: Vec<String>,
}

pub(super) fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

pub(super) fn state_path(dir: &Path) -> PathBuf {
    dir.join("soak.json")
}

pub(super) fn rounds_path(dir: &Path) -> PathBuf {
    dir.join("rounds.jsonl")
}

pub(super) fn read_state(dir: &Path) -> anyhow::Result<SoakState> {
    let path = state_path(dir);
    let state: SoakState = serde_json::from_slice(
        &fs::read(&path)
            .with_context(|| format!("no soak at {} (run `soak init`)", dir.display()))?,
    )?;
    anyhow::ensure!(
        state.schema_version == SCHEMA_VERSION,
        "unsupported soak schema {}",
        state.schema_version
    );
    Ok(state)
}

/// Replace the state atomically: a crash leaves the old or the new state, never a torn one.
pub(super) fn write_state(dir: &Path, state: &SoakState) -> anyhow::Result<()> {
    let path = state_path(dir);
    let temporary = path.with_extension("tmp");
    let mut file = fs::File::create(&temporary)?;
    serde_json::to_writer_pretty(&mut file, state)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    fs::rename(&temporary, &path)?;
    fs::File::open(dir)?.sync_all()?;
    Ok(())
}

/// Append one record and sync it, so a finished round is never lost to a later crash.
pub(super) fn append_round(dir: &Path, record: &RoundRecord) -> anyhow::Result<()> {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(rounds_path(dir))?;
    serde_json::to_writer(&mut file, record)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

pub(super) fn read_rounds(dir: &Path) -> anyhow::Result<Vec<RoundRecord>> {
    let path = rounds_path(dir);
    if !path.exists() {
        return Ok(Vec::new());
    }
    fs::read_to_string(&path)?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).context("a rounds.jsonl line is not a round record"))
        .collect()
}
