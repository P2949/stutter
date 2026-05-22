use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct AutotuneRestoreCommandInput {
    pub journal_path: Option<PathBuf>,
    pub audit_path: Option<PathBuf>,
    pub history_path: Option<PathBuf>,
    pub dry_run: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutotuneRestoreOutcome {
    pub status: AutotuneRestoreStatus,
    pub restored_actions: usize,
    pub failed_actions: usize,
    pub skipped_actions: usize,
    pub restored_records: usize,
    pub skipped_missing: usize,
    pub skipped_identity_mismatch: usize,
    pub failed_records: usize,
    pub messages: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AutotuneRestoreStatus {
    Clean,
    ApplyingWithoutRollbackToken,
    DryRun,
    Restored,
    Faulted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RollbackRestoreSummary {
    pub rollback_kind: String,
    pub restored_items: usize,
    pub skipped_items: usize,
    pub skipped_missing: usize,
    pub skipped_identity_mismatch: usize,
    pub failed_items: usize,
    pub messages: Vec<String>,
}

impl RollbackRestoreSummary {
    pub fn success(rollback_kind: impl Into<String>, restored_items: usize) -> Self {
        Self {
            rollback_kind: rollback_kind.into(),
            restored_items,
            skipped_items: 0,
            skipped_missing: 0,
            skipped_identity_mismatch: 0,
            failed_items: 0,
            messages: Vec::new(),
        }
    }

    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.messages.push(message.into());
        self
    }
}
