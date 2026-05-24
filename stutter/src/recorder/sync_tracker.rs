//! Directory fsync bookkeeping for recording artifact writers.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::Context;

#[derive(Debug, Default)]
pub struct SyncTracker {
    synced_dirs: BTreeSet<PathBuf>,
}

impl SyncTracker {
    pub fn sync_parent_once(&mut self, path: &Path) -> anyhow::Result<()> {
        let Some(parent) = path.parent() else {
            return Ok(());
        };

        let parent = parent.to_path_buf();

        if self.synced_dirs.insert(parent.clone()) {
            let dir = fs::File::open(&parent).with_context(|| {
                format!(
                    "failed to open parent directory {} for sync",
                    parent.display()
                )
            })?;

            dir.sync_all()
                .with_context(|| format!("failed to sync parent directory {}", parent.display()))?;
        }

        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn synced_dir_count_for_test(&self) -> usize {
        self.synced_dirs.len()
    }

    #[cfg(test)]
    pub(crate) fn mark_parent_for_test(&mut self, path: &Path) {
        if let Some(parent) = path.parent() {
            self.synced_dirs.insert(parent.to_path_buf());
        }
    }
}
