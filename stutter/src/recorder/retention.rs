use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use anyhow::Context;

use crate::{
    artifacts::{ArtifactKind, artifact_path},
    config::model::RecordingConfig,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecordingRetentionPolicy {
    pub max_run_count: Option<usize>,
    pub max_total_bytes: Option<u64>,
    pub max_age_seconds: Option<u64>,
    pub min_free_bytes: Option<u64>,
}

impl RecordingRetentionPolicy {
    pub fn from_recording_config(config: &RecordingConfig) -> Self {
        Self {
            max_run_count: config.retention.max_run_count,
            max_total_bytes: config.retention.max_total_bytes,
            max_age_seconds: config.retention.max_age_seconds,
            min_free_bytes: config.retention.min_free_bytes,
        }
    }

    pub fn has_prune_budget(&self) -> bool {
        self.max_run_count.is_some()
            || self.max_total_bytes.is_some()
            || self.max_age_seconds.is_some()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecordingRetentionSummary {
    pub scanned_runs: usize,
    pub deleted_runs: usize,
    pub deleted_bytes: u64,
    pub remaining_runs: usize,
    pub remaining_bytes: u64,
}

#[derive(Clone, Debug)]
struct RetentionRun {
    path: PathBuf,
    modified: SystemTime,
    size_bytes: u64,
}

pub fn apply_recording_retention(
    root: &Path,
    policy: &RecordingRetentionPolicy,
    protected_path: Option<&Path>,
    now: SystemTime,
) -> anyhow::Result<RecordingRetentionSummary> {
    if !policy.has_prune_budget() || !root.exists() {
        return Ok(RecordingRetentionSummary::default());
    }

    let protected_path = protected_path.and_then(|path| path.canonicalize().ok());
    let mut runs = discover_retention_runs(root)?;
    let scanned_runs = runs.len();
    let mut deleted_runs = 0usize;
    let mut deleted_bytes = 0u64;

    if let Some(max_age_seconds) = policy.max_age_seconds {
        let max_age = Duration::from_secs(max_age_seconds);
        let mut retained = Vec::with_capacity(runs.len());
        for run in runs {
            let protected = path_is_protected(&run.path, protected_path.as_deref());
            let expired = now
                .duration_since(run.modified)
                .is_ok_and(|age| age > max_age);

            if expired && !protected {
                delete_run(&run)?;
                deleted_runs += 1;
                deleted_bytes = deleted_bytes.saturating_add(run.size_bytes);
            } else {
                retained.push(run);
            }
        }
        runs = retained;
    }

    sort_newest_first(&mut runs);

    if let Some(max_run_count) = policy.max_run_count {
        let mut retained = Vec::with_capacity(runs.len().min(max_run_count));
        for (index, run) in runs.into_iter().enumerate() {
            let protected = path_is_protected(&run.path, protected_path.as_deref());
            if index >= max_run_count && !protected {
                delete_run(&run)?;
                deleted_runs += 1;
                deleted_bytes = deleted_bytes.saturating_add(run.size_bytes);
            } else {
                retained.push(run);
            }
        }
        runs = retained;
    }

    if let Some(max_total_bytes) = policy.max_total_bytes {
        let mut total = runs.iter().map(|run| run.size_bytes).sum::<u64>();
        sort_oldest_first(&mut runs);
        let mut retained = Vec::with_capacity(runs.len());

        for run in runs {
            let protected = path_is_protected(&run.path, protected_path.as_deref());
            if total > max_total_bytes && !protected {
                delete_run(&run)?;
                total = total.saturating_sub(run.size_bytes);
                deleted_runs += 1;
                deleted_bytes = deleted_bytes.saturating_add(run.size_bytes);
            } else {
                retained.push(run);
            }
        }

        runs = retained;
    }

    let remaining_runs = runs.len();
    let remaining_bytes = runs.iter().map(|run| run.size_bytes).sum();

    Ok(RecordingRetentionSummary {
        scanned_runs,
        deleted_runs,
        deleted_bytes,
        remaining_runs,
        remaining_bytes,
    })
}

pub fn ensure_min_free_space_for_path(path: &Path, min_free_bytes: u64) -> anyhow::Result<()> {
    let available = available_bytes_for_path(path)
        .with_context(|| format!("failed to inspect free disk space for {}", path.display()))?;
    ensure_min_free_space_from_available(path, available, min_free_bytes)
}

fn ensure_min_free_space_from_available(
    path: &Path,
    available_bytes: u64,
    min_free_bytes: u64,
) -> anyhow::Result<()> {
    if available_bytes < min_free_bytes {
        anyhow::bail!(
            "recording emergency stop: free disk space at {} is {} bytes, below configured minimum {} bytes",
            path.display(),
            available_bytes,
            min_free_bytes
        );
    }

    Ok(())
}

fn discover_retention_runs(root: &Path) -> anyhow::Result<Vec<RetentionRun>> {
    let mut runs = Vec::new();

    for entry in fs::read_dir(root)
        .with_context(|| format!("failed to scan recording run directory {}", root.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_dir() || !looks_like_stutter_run_dir(&path) {
            continue;
        }

        runs.push(RetentionRun {
            modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            size_bytes: directory_size_bytes(&path)?,
            path,
        });
    }

    Ok(runs)
}

fn looks_like_stutter_run_dir(path: &Path) -> bool {
    if artifact_path(path, ArtifactKind::Session).is_file()
        || artifact_path(path, ArtifactKind::Metadata).is_file()
    {
        return true;
    }

    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(looks_like_timestamped_run_name)
}

fn looks_like_timestamped_run_name(name: &str) -> bool {
    let mut parts = name.splitn(3, '_');
    let Some(seconds) = parts.next() else {
        return false;
    };
    let Some(nanos) = parts.next() else {
        return false;
    };
    let Some(label) = parts.next() else {
        return false;
    };

    !label.is_empty()
        && seconds.chars().all(|ch| ch.is_ascii_digit())
        && nanos.len() == 9
        && nanos.chars().all(|ch| ch.is_ascii_digit())
}

fn directory_size_bytes(path: &Path) -> anyhow::Result<u64> {
    let mut total = 0u64;
    for entry in fs::read_dir(path)
        .with_context(|| format!("failed to size recording directory {}", path.display()))?
    {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.is_dir() {
            total = total.saturating_add(directory_size_bytes(&entry.path())?);
        } else {
            total = total.saturating_add(metadata.len());
        }
    }

    Ok(total)
}

fn delete_run(run: &RetentionRun) -> anyhow::Result<()> {
    fs::remove_dir_all(&run.path)
        .with_context(|| format!("failed to delete old recording run {}", run.path.display()))
}

fn path_is_protected(path: &Path, protected_path: Option<&Path>) -> bool {
    let Some(protected_path) = protected_path else {
        return false;
    };
    path.canonicalize()
        .ok()
        .as_deref()
        .is_some_and(|path| path == protected_path)
}

fn sort_newest_first(runs: &mut [RetentionRun]) {
    runs.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| a.path.cmp(&b.path))
    });
}

fn sort_oldest_first(runs: &mut [RetentionRun]) {
    runs.sort_by(|a, b| {
        a.modified
            .cmp(&b.modified)
            .then_with(|| a.path.cmp(&b.path))
    });
}

fn available_bytes_for_path(path: &Path) -> io::Result<u64> {
    let existing_path = nearest_existing_path(path);
    crate::syscall::statvfs(&existing_path).map(|space| space.free_bytes)
}

fn nearest_existing_path(path: &Path) -> PathBuf {
    let mut current = path;
    loop {
        if current.exists() {
            return current.to_path_buf();
        }
        let Some(parent) = current.parent() else {
            return PathBuf::from(".");
        };
        current = parent;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-retention-test-{name}-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn make_run(root: &Path, name: &str, bytes: usize) -> PathBuf {
        let path = root.join(name);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("session.json"), b"{}\n").unwrap();
        fs::write(path.join("payload.bin"), vec![b'x'; bytes]).unwrap();
        path
    }

    #[test]
    fn timestamped_run_name_detection_is_conservative() {
        assert!(looks_like_timestamped_run_name("1770000000_000000001_run"));
        assert!(!looks_like_timestamped_run_name("notes"));
        assert!(!looks_like_timestamped_run_name("1770000000_1_run"));
        assert!(!looks_like_timestamped_run_name("1770000000_000000001_"));
    }

    #[test]
    fn retention_prunes_oldest_runs_by_count_without_touching_unrelated_dirs() {
        let root = temp_dir("count");
        let old = make_run(&root, "1000000000_000000000_old", 10);
        std::thread::sleep(Duration::from_millis(2));
        let keep = make_run(&root, "2000000000_000000000_keep", 10);
        let notes = root.join("notes");
        fs::create_dir_all(&notes).unwrap();
        fs::write(notes.join("payload.bin"), b"do not delete").unwrap();

        let summary = apply_recording_retention(
            &root,
            &RecordingRetentionPolicy {
                max_run_count: Some(1),
                ..RecordingRetentionPolicy::default()
            },
            None,
            SystemTime::now(),
        )
        .unwrap();

        assert_eq!(summary.scanned_runs, 2);
        assert_eq!(summary.deleted_runs, 1);
        assert!(!old.exists());
        assert!(keep.exists());
        assert!(notes.exists());

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn retention_prunes_by_total_bytes_but_keeps_protected_run() {
        let root = temp_dir("bytes");
        let old = make_run(&root, "1000000000_000000000_old", 64);
        std::thread::sleep(Duration::from_millis(2));
        let current = make_run(&root, "2000000000_000000000_current", 128);

        let summary = apply_recording_retention(
            &root,
            &RecordingRetentionPolicy {
                max_total_bytes: Some(32),
                ..RecordingRetentionPolicy::default()
            },
            Some(&current),
            SystemTime::now(),
        )
        .unwrap();

        assert_eq!(summary.deleted_runs, 1);
        assert!(!old.exists());
        assert!(current.exists());

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn retention_prunes_by_age() {
        let root = temp_dir("age");
        let run = make_run(&root, "1000000000_000000000_old", 10);

        let summary = apply_recording_retention(
            &root,
            &RecordingRetentionPolicy {
                max_age_seconds: Some(0),
                ..RecordingRetentionPolicy::default()
            },
            None,
            SystemTime::now() + Duration::from_secs(1),
        )
        .unwrap();

        assert_eq!(summary.deleted_runs, 1);
        assert!(!run.exists());

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn low_disk_preflight_rejects_below_configured_minimum() {
        let err = ensure_min_free_space_from_available(Path::new("/tmp"), 99, 100).unwrap_err();

        assert!(err.to_string().contains("recording emergency stop"));
    }
}
