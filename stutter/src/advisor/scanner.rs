use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use crate::artifacts::{ArtifactKind, artifact_path};

pub fn default_runs_dir() -> PathBuf {
    let mut path = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    path.push(".local");
    path.push("state");
    path.push("stutter");
    path.push("runs");
    path
}

pub fn completed_run_dirs(
    runs_dir: &Path,
    processed: &BTreeSet<PathBuf>,
) -> anyhow::Result<Vec<PathBuf>> {
    completed_run_dirs_with_min_age(runs_dir, processed, Duration::from_secs(2))
}

pub fn completed_run_dirs_with_min_age(
    runs_dir: &Path,
    processed: &BTreeSet<PathBuf>,
    min_session_age: Duration,
) -> anyhow::Result<Vec<PathBuf>> {
    if !runs_dir.exists() {
        return Ok(Vec::new());
    }
    let mut runs = Vec::new();
    for entry in fs::read_dir(runs_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() || processed.contains(&path) {
            continue;
        }
        let session_path = artifact_path(&path, ArtifactKind::Session);
        if !session_path.exists() {
            continue;
        }
        let modified = session_path
            .metadata()?
            .modified()
            .unwrap_or(SystemTime::UNIX_EPOCH);
        if modified.elapsed().unwrap_or(Duration::ZERO) < min_session_age {
            continue;
        }
        runs.push(path);
    }
    runs.sort();
    Ok(runs)
}
