use std::{fs, path::{Path, PathBuf}};
use crate::{autotune::emergency_restore::*};

    pub(super) fn temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-emergency-restore-test-{name}-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    pub(super) fn command_input_for_dir(dir: &Path, dry_run: bool) -> AutotuneRestoreCommandInput {
        AutotuneRestoreCommandInput {
            journal_path: Some(dir.join("controller_journal.json")),
            audit_path: Some(dir.join("audit.jsonl")),
            history_path: Some(dir.join("history.jsonl")),
            dry_run,
        }
    }
