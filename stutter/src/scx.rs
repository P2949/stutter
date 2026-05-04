use std::{fs, path::Path};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScxEvent {
    pub elapsed_ms: u128,
    pub state: Option<String>,
    pub ops: Option<String>,
    pub enable_seq: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ScxTracker {
    last: Option<ScxSnapshot>,
    events: Vec<ScxEvent>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ScxSnapshot {
    state: Option<String>,
    ops: Option<String>,
    enable_seq: Option<String>,
}

impl ScxTracker {
    pub fn sample(&mut self, elapsed_ms: u128) {
        self.sample_at(Path::new("/sys/kernel/sched_ext"), elapsed_ms);
    }
    #[allow(dead_code)]
    pub fn events(&self) -> &[ScxEvent] {
        &self.events
    }

    #[cfg(test)]
    pub fn sample_at(&mut self, root: &Path, elapsed_ms: u128) {
        self.record_snapshot(snapshot_at(root), elapsed_ms);
    }

    #[cfg(not(test))]
    fn sample_at(&mut self, root: &Path, elapsed_ms: u128) {
        self.record_snapshot(snapshot_at(root), elapsed_ms);
    }

    fn record_snapshot(&mut self, snapshot: ScxSnapshot, elapsed_ms: u128) {
        if snapshot.is_empty() {
            return;
        }

        if self.last.as_ref() == Some(&snapshot) {
            return;
        }

        self.events.push(ScxEvent {
            elapsed_ms,
            state: snapshot.state.clone(),
            ops: snapshot.ops.clone(),
            enable_seq: snapshot.enable_seq.clone(),
        });
        self.last = Some(snapshot);
    }
}

impl ScxSnapshot {
    fn is_empty(&self) -> bool {
        self.state.is_none() && self.ops.is_none() && self.enable_seq.is_none()
    }
}

fn snapshot_at(root: &Path) -> ScxSnapshot {
    ScxSnapshot {
        state: read_trimmed(root.join("state")),
        ops: read_trimmed(root.join("root/ops")),
        enable_seq: read_trimmed(root.join("enable_seq")),
    }
}

fn read_trimmed(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn records_only_scx_changes() {
        let root = temp_dir("scx");
        fs::create_dir_all(root.join("root")).unwrap();
        fs::write(root.join("state"), "enabled\n").unwrap();
        fs::write(root.join("root/ops"), "scx_lavd\n").unwrap();
        fs::write(root.join("enable_seq"), "1\n").unwrap();

        let mut tracker = ScxTracker::default();
        tracker.sample_at(&root, 0);
        tracker.sample_at(&root, 1_000);
        fs::write(root.join("root/ops"), "scx_p2dq\n").unwrap();
        tracker.sample_at(&root, 2_000);

        assert_eq!(tracker.events().len(), 2);
        assert_eq!(tracker.events()[0].ops.as_deref(), Some("scx_lavd"));
        assert_eq!(tracker.events()[1].ops.as_deref(), Some("scx_p2dq"));

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn skips_empty_scx_snapshot() {
        let root = temp_dir("scx-empty");
        fs::create_dir_all(&root).unwrap();

        let mut tracker = ScxTracker::default();
        tracker.sample_at(&root, 0);

        assert!(tracker.events().is_empty());
        fs::remove_dir_all(root).ok();
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        dir
    }
}
