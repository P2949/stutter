//! Root-owned runner state: reboot budget, cancellation and the resume flag. Corrupt state is
//! reported as such and stops privileged actions that could reboot; it is never "repaired".
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::model::SCHEMA_VERSION;

pub fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct RunnerState {
    pub schema_version: u32,
    pub campaign_id: String,
    pub reboots_used: u32,
    pub cancelled: bool,
    /// Set when an authorized reboot is under way so the resume service continues the campaign.
    pub resume_pending: bool,
    /// Boot entry selected for the next boot, if any.
    pub pending_boot_entry: Option<String>,
    pub updated_unix_ms: u64,
}

#[derive(Debug)]
pub enum StateError {
    Corrupt(String),
    Io(std::io::Error),
}

impl std::fmt::Display for StateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Corrupt(why) => write!(f, "runner state is corrupt: {why}"),
            Self::Io(error) => write!(f, "runner state i/o error: {error}"),
        }
    }
}

pub struct StateDir(pub PathBuf);

impl StateDir {
    pub fn state_path(&self) -> PathBuf {
        self.0.join("boot-state.json")
    }
    pub fn receipts(&self) -> PathBuf {
        self.0.join("privileged-receipts")
    }

    /// A missing state file is a fresh campaign; an unreadable or malformed one is corrupt.
    pub fn load(&self, campaign_id: &str) -> Result<RunnerState, StateError> {
        let path = self.state_path();
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(RunnerState {
                    schema_version: SCHEMA_VERSION,
                    campaign_id: campaign_id.to_owned(),
                    ..RunnerState::default()
                });
            }
            Err(error) => return Err(StateError::Io(error)),
        };
        let state: RunnerState = serde_json::from_slice(&bytes)
            .map_err(|error| StateError::Corrupt(error.to_string()))?;
        if state.schema_version != SCHEMA_VERSION || state.campaign_id != campaign_id {
            return Err(StateError::Corrupt(
                "schema or campaign id mismatch".to_owned(),
            ));
        }
        Ok(state)
    }

    pub fn save(&self, state: &RunnerState) -> std::io::Result<()> {
        let mut state = state.clone();
        state.updated_unix_ms = unix_ms();
        atomic_write(&self.state_path(), &serde_json::to_vec_pretty(&state)?)
    }

    pub fn write_receipt(
        &self,
        request_id: &str,
        body: &serde_json::Value,
    ) -> std::io::Result<String> {
        let id = format!("{request_id}.json");
        atomic_write(
            &self.receipts().join(&id),
            &serde_json::to_vec_pretty(body)?,
        )?;
        Ok(id)
    }
}

/// Temp file, `sync_all`, rename, parent directory sync.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "no parent"))?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("state"),
        std::process::id()
    ));
    let mut file = fs::File::create(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&temporary, path)?;
    fs::File::open(parent)?.sync_all()
}
