//! Recheck the configured study root against the independently frozen host identity.
use std::{fs::File, os::unix::fs::MetadataExt, path::PathBuf};

use super::ResearchCoordinator;
use crate::autotune::research::CampaignFacts;

impl ResearchCoordinator {
    pub(in crate::autotune::runtime) fn set_study_dir(&mut self, path: Option<PathBuf>) {
        self.study_dir = path;
    }

    pub(super) fn study_root_matches(&self, facts: &CampaignFacts) -> bool {
        let Some(expected) = facts.study_root else {
            // Historical prelaunch records can be read, but current host facts always include it.
            return self.study_dir.is_none();
        };
        let Some(path) = &self.study_dir else {
            return false;
        };
        let Ok(dir) = crate::syscall::open_dir_no_symlinks(path) else {
            return false;
        };
        let Ok(meta) = File::from(dir).metadata() else {
            return false;
        };
        meta.is_dir()
            && meta.uid() == facts.uid
            && meta.mode() & 0o077 == 0
            && meta.dev() == expected.device
            && meta.ino() == expected.inode
    }
}
