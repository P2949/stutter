use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::actions::RollbackToken;

pub const CONTROLLER_JOURNAL_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ControllerJournalState {
    Applying,
    Applied,
    Clean,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ControllerJournalRecord {
    Applying {
        schema_version: u32,
        experiment_id: String,
        action_id: String,
        rollback_token: Option<RollbackToken>,
    },
    Applied {
        schema_version: u32,
        experiment_id: String,
        action_id: String,
        rollback_token: RollbackToken,
    },
    Clean {
        schema_version: u32,
    },
}

impl ControllerJournalRecord {
    pub fn applying(experiment_id: impl Into<String>, action_id: impl Into<String>) -> Self {
        Self::Applying {
            schema_version: CONTROLLER_JOURNAL_SCHEMA_VERSION,
            experiment_id: experiment_id.into(),
            action_id: action_id.into(),
            rollback_token: None,
        }
    }

    pub fn applied(
        experiment_id: impl Into<String>,
        action_id: impl Into<String>,
        rollback_token: RollbackToken,
    ) -> Self {
        Self::Applied {
            schema_version: CONTROLLER_JOURNAL_SCHEMA_VERSION,
            experiment_id: experiment_id.into(),
            action_id: action_id.into(),
            rollback_token,
        }
    }

    pub fn clean() -> Self {
        Self::Clean {
            schema_version: CONTROLLER_JOURNAL_SCHEMA_VERSION,
        }
    }

    pub fn schema_version(&self) -> u32 {
        match self {
            Self::Applying { schema_version, .. }
            | Self::Applied { schema_version, .. }
            | Self::Clean { schema_version } => *schema_version,
        }
    }

    pub fn state(&self) -> ControllerJournalState {
        match self {
            Self::Applying { .. } => ControllerJournalState::Applying,
            Self::Applied { .. } => ControllerJournalState::Applied,
            Self::Clean { .. } => ControllerJournalState::Clean,
        }
    }

    pub fn is_clean(&self) -> bool {
        matches!(self, Self::Clean { .. })
    }
}

pub fn default_controller_journal_path() -> PathBuf {
    let mut path = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    path.push(".local");
    path.push("state");
    path.push("stutter");
    path.push("autotune");
    path.push("controller_journal.json");
    path
}

pub fn write_controller_journal_record(
    path: &Path,
    record: &ControllerJournalRecord,
) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create autotune controller journal directory {}",
                parent.display()
            )
        })?;
    }

    let temporary_path = temporary_journal_path(path);
    {
        let mut file = fs::File::create(&temporary_path).with_context(|| {
            format!(
                "failed to create autotune controller journal temp file {}",
                temporary_path.display()
            )
        })?;

        serde_json::to_writer_pretty(&mut file, record).with_context(|| {
            format!(
                "failed to serialize autotune controller journal {}",
                temporary_path.display()
            )
        })?;
        file.write_all(b"\n").with_context(|| {
            format!(
                "failed to terminate autotune controller journal {}",
                temporary_path.display()
            )
        })?;
        file.sync_all().with_context(|| {
            format!(
                "failed to sync autotune controller journal temp file {}",
                temporary_path.display()
            )
        })?;
    }

    fs::rename(&temporary_path, path).with_context(|| {
        format!(
            "failed to atomically replace autotune controller journal {} with {}",
            path.display(),
            temporary_path.display()
        )
    })?;

    Ok(())
}

pub fn read_controller_journal(path: &Path) -> anyhow::Result<ControllerJournalRecord> {
    if !path.exists() {
        return Ok(ControllerJournalRecord::clean());
    }

    let file = fs::File::open(path).with_context(|| {
        format!(
            "failed to open autotune controller journal {}",
            path.display()
        )
    })?;

    let record: ControllerJournalRecord = serde_json::from_reader(file).with_context(|| {
        format!(
            "failed to parse autotune controller journal {}",
            path.display()
        )
    })?;

    if record.schema_version() != CONTROLLER_JOURNAL_SCHEMA_VERSION {
        anyhow::bail!(
            "unsupported autotune controller journal schema_version={} in {}",
            record.schema_version(),
            path.display()
        );
    }

    Ok(record)
}

pub fn write_controller_journal_applying(
    path: &Path,
    experiment_id: &str,
    action_id: &str,
) -> anyhow::Result<ControllerJournalRecord> {
    let record = ControllerJournalRecord::applying(experiment_id, action_id);
    write_controller_journal_record(path, &record)?;
    Ok(record)
}

pub fn write_controller_journal_applied(
    path: &Path,
    experiment_id: &str,
    action_id: &str,
    rollback_token: RollbackToken,
) -> anyhow::Result<ControllerJournalRecord> {
    let record = ControllerJournalRecord::applied(experiment_id, action_id, rollback_token);
    write_controller_journal_record(path, &record)?;
    Ok(record)
}

pub fn write_controller_journal_clean(path: &Path) -> anyhow::Result<ControllerJournalRecord> {
    let record = ControllerJournalRecord::clean();
    write_controller_journal_record(path, &record)?;
    Ok(record)
}

pub fn write_default_controller_journal_applying(
    experiment_id: &str,
    action_id: &str,
) -> anyhow::Result<PathBuf> {
    let path = default_controller_journal_path();
    write_controller_journal_applying(&path, experiment_id, action_id)?;
    Ok(path)
}

pub fn write_default_controller_journal_applied(
    experiment_id: &str,
    action_id: &str,
    rollback_token: RollbackToken,
) -> anyhow::Result<PathBuf> {
    let path = default_controller_journal_path();
    write_controller_journal_applied(&path, experiment_id, action_id, rollback_token)?;
    Ok(path)
}

pub fn write_default_controller_journal_clean() -> anyhow::Result<PathBuf> {
    let path = default_controller_journal_path();
    write_controller_journal_clean(&path)?;
    Ok(path)
}

fn temporary_journal_path(path: &Path) -> PathBuf {
    let mut temporary_path = path.to_path_buf();
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("controller_journal.json");
    temporary_path.set_file_name(format!("{file_name}.tmp"));
    temporary_path
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-controller-journal-test-{name}-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn rollback_token() -> RollbackToken {
        RollbackToken::CpuAffinityRestoreFile {
            path: PathBuf::from("/tmp/stutter-restore.json"),
            affected_tasks: 31,
        }
    }

    #[test]
    fn applying_journal_serializes_with_null_rollback_token() {
        let record =
            ControllerJournalRecord::applying("experiment-1", "cpu-affinity-profile:game-main");

        let value = serde_json::to_value(&record).unwrap();

        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["state"], "applying");
        assert_eq!(value["experiment_id"], "experiment-1");
        assert_eq!(value["action_id"], "cpu-affinity-profile:game-main");
        assert!(value["rollback_token"].is_null());
        assert_eq!(record.state(), ControllerJournalState::Applying);
    }

    #[test]
    fn applied_journal_serializes_with_rollback_token() {
        let record = ControllerJournalRecord::applied(
            "experiment-1",
            "cpu-affinity-profile:game-main",
            rollback_token(),
        );

        let value = serde_json::to_value(&record).unwrap();

        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["state"], "applied");
        assert_eq!(value["experiment_id"], "experiment-1");
        assert_eq!(value["action_id"], "cpu-affinity-profile:game-main");
        assert_eq!(value["rollback_token"]["kind"], "cpu-affinity-restore-file");
        assert_eq!(value["rollback_token"]["affected_tasks"], 31);
        assert_eq!(record.state(), ControllerJournalState::Applied);
    }

    #[test]
    fn clean_journal_serializes_without_action_fields() {
        let record = ControllerJournalRecord::clean();

        let value = serde_json::to_value(&record).unwrap();

        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["state"], "clean");
        assert!(value.get("experiment_id").is_none());
        assert!(value.get("action_id").is_none());
        assert!(value.get("rollback_token").is_none());
        assert!(record.is_clean());
    }

    #[test]
    fn journal_round_trips_and_atomic_write_removes_temp_file() {
        let dir = temp_dir("round-trip");
        let path = dir.join("controller_journal.json");

        let applying = write_controller_journal_applying(
            &path,
            "experiment-1",
            "cpu-affinity-profile:game-main",
        )
        .unwrap();
        assert_eq!(read_controller_journal(&path).unwrap(), applying);

        let applied = write_controller_journal_applied(
            &path,
            "experiment-1",
            "cpu-affinity-profile:game-main",
            rollback_token(),
        )
        .unwrap();
        assert_eq!(read_controller_journal(&path).unwrap(), applied);

        let clean = write_controller_journal_clean(&path).unwrap();
        assert_eq!(read_controller_journal(&path).unwrap(), clean);
        assert!(!temporary_journal_path(&path).exists());

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn missing_journal_reads_as_clean() {
        let dir = temp_dir("missing");
        let path = dir.join("missing-controller-journal.json");

        let record = read_controller_journal(&path).unwrap();

        assert_eq!(record, ControllerJournalRecord::clean());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn default_controller_journal_path_matches_autotune_state_directory() {
        let path = default_controller_journal_path();
        let rendered = path.to_string_lossy();

        assert!(rendered.ends_with(".local/state/stutter/autotune/controller_journal.json"));
    }
}
