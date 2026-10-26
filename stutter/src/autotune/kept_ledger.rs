//! Durable record of every kept action's rollback token.
//!
//! The controller journal holds one record: the experiment in flight. A kept action is applied
//! after its experiment ends, and the next experiment replaces the journal, so without this ledger
//! a controller crash would lose the token of every earlier kept action. An entry is written
//! (synced) before the action counts as kept and retired only after its rollback succeeded, so a
//! failed restore stays on disk until startup recovery or the operator can finish it.
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::{
    actions::{ActionId, RollbackToken, SafetyClass},
    autotune::experiment::ExperimentId,
    daemon::policy::DaemonMode,
};

pub const KEPT_LEDGER_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeptLedgerEntry {
    pub experiment_id: ExperimentId,
    pub action_id: ActionId,
    pub candidate: String,
    pub rollback_token: RollbackToken,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<DaemonMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safety_class: Option<SafetyClass>,
    pub kept_unix_nanos: u128,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeptLedger {
    pub schema_version: u32,
    pub entries: Vec<KeptLedgerEntry>,
}

impl Default for KeptLedger {
    fn default() -> Self {
        Self {
            schema_version: KEPT_LEDGER_SCHEMA_VERSION,
            entries: Vec::new(),
        }
    }
}

/// The ledger lives beside the controller journal it complements.
pub fn kept_ledger_path(journal_path: &Path) -> PathBuf {
    journal_path.with_file_name("kept-actions.json")
}

/// A missing ledger is empty; an unreadable one is an error, never silently empty.
pub fn read_kept_ledger(path: &Path) -> anyhow::Result<KeptLedger> {
    if !path.exists() {
        return Ok(KeptLedger::default());
    }
    let bytes = fs::read(path)
        .with_context(|| format!("failed to read kept-action ledger {}", path.display()))?;
    let ledger: KeptLedger = serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse kept-action ledger {}", path.display()))?;
    anyhow::ensure!(
        ledger.schema_version == KEPT_LEDGER_SCHEMA_VERSION,
        "unsupported kept-action ledger schema {}",
        ledger.schema_version
    );
    Ok(ledger)
}

fn write_kept_ledger(path: &Path, ledger: &KeptLedger) -> anyhow::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    let mut file = fs::File::create(&temporary)
        .with_context(|| format!("failed to create {}", temporary.display()))?;
    serde_json::to_writer_pretty(&mut file, ledger)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary, path)
        .with_context(|| format!("failed to replace kept-action ledger {}", path.display()))?;
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

/// Record a kept action. Idempotent per experiment: a repeat replaces the earlier entry.
pub fn record_kept(path: &Path, entry: KeptLedgerEntry) -> anyhow::Result<()> {
    let mut ledger = read_kept_ledger(path)?;
    ledger
        .entries
        .retain(|existing| existing.experiment_id != entry.experiment_id);
    ledger.entries.push(entry);
    write_kept_ledger(path, &ledger)
}

/// Retire an entry once its rollback verifiably succeeded. Retiring an absent entry is a no-op.
pub fn retire_kept(path: &Path, experiment_id: &ExperimentId) -> anyhow::Result<()> {
    let mut ledger = read_kept_ledger(path)?;
    let before = ledger.entries.len();
    ledger
        .entries
        .retain(|existing| &existing.experiment_id != experiment_id);
    if ledger.entries.len() == before {
        return Ok(());
    }
    if ledger.entries.is_empty() {
        fs::remove_file(path).or_else(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                Ok(())
            } else {
                Err(error)
            }
        })?;
        fs::File::open(path.parent().unwrap_or_else(|| Path::new(".")))?.sync_all()?;
        return Ok(());
    }
    write_kept_ledger(path, &ledger)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str) -> KeptLedgerEntry {
        KeptLedgerEntry {
            experiment_id: ExperimentId::new(name),
            action_id: ActionId::new(format!("action-{name}")),
            candidate: name.to_owned(),
            rollback_token: RollbackToken::NiceRestore {
                records: Vec::new(),
            },
            mode: None,
            safety_class: None,
            kept_unix_nanos: 1,
        }
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "stutter-kept-ledger-{name}-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir.join("kept-actions.json")
    }

    #[test]
    fn entries_accumulate_are_idempotent_and_retire_individually() {
        let path = temp("accumulate");
        assert!(read_kept_ledger(&path).unwrap().entries.is_empty());
        record_kept(&path, entry("a")).unwrap();
        record_kept(&path, entry("b")).unwrap();
        record_kept(&path, entry("a")).unwrap();
        assert_eq!(read_kept_ledger(&path).unwrap().entries.len(), 2);
        retire_kept(&path, &ExperimentId::new("a")).unwrap();
        let remaining = read_kept_ledger(&path).unwrap().entries;
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].experiment_id, ExperimentId::new("b"));
        retire_kept(&path, &ExperimentId::new("b")).unwrap();
        assert!(!path.exists(), "an empty ledger leaves no file behind");
        retire_kept(&path, &ExperimentId::new("b")).unwrap();
    }

    #[test]
    fn a_corrupt_ledger_is_an_error_not_an_empty_ledger() {
        let path = temp("corrupt");
        fs::write(&path, b"{ not json").unwrap();
        assert!(read_kept_ledger(&path).is_err());
        assert!(record_kept(&path, entry("a")).is_err());
    }

    #[test]
    fn the_ledger_path_sits_beside_the_journal() {
        assert_eq!(
            kept_ledger_path(Path::new("/x/y/controller_journal.json")),
            PathBuf::from("/x/y/kept-actions.json")
        );
    }
}
