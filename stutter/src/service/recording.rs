#![allow(dead_code)] // Transitional service boundary; agent/CLI call sites migrate incrementally.

use std::path::PathBuf;

#[derive(Clone, Debug)]
pub(crate) struct RecordingService {
    runs_dir: PathBuf,
}

impl RecordingService {
    pub(crate) fn new(runs_dir: PathBuf) -> Self {
        Self { runs_dir }
    }

    pub(crate) fn runs_dir(&self) -> &PathBuf {
        &self.runs_dir
    }
}
