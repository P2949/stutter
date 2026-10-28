//! Reading the controller's own history: the decisions that matter to a soak (everything but the
//! per-interval `observed` records) and the audit log.
use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct HistoryEvent {
    pub decision: String,
    pub action_kind: Option<String>,
    pub experiment_id: Option<String>,
    pub action_id: Option<String>,
    pub situation: String,
    pub controller_id: String,
    pub mode: String,
    pub unix_nanos: u128,
    pub rollback_performed: bool,
    pub reason: String,
}

impl HistoryEvent {
    /// A keep or a revert of an experiment. A revert forced by the runtime (suspend, workload
    /// exit, a stop) is recorded as the next decision carrying the rollback, not as
    /// `candidate_reverted`, and that context repeats on later records: count each experiment once
    /// (see `first_resolutions`).
    pub fn is_resolution(&self) -> bool {
        matches!(
            self.decision.as_str(),
            "candidate_kept" | "candidate_reverted"
        ) || (self.rollback_performed
            && self.experiment_id.is_some()
            && self.controller_id == "local-autotune")
    }

    pub fn is_keep(&self) -> bool {
        self.decision == "candidate_kept"
    }
}

/// `None` for the per-interval `observed` records and for anything that is not a history line.
pub fn parse_history_line(line: &str) -> Option<HistoryEvent> {
    // The observed records are about 99% of the file: skip them without parsing.
    if line.contains("\"decision\":\"observed\"") {
        return None;
    }
    let value: Value = serde_json::from_str(line).ok()?;
    let text = |pointer: &str| {
        value
            .pointer(pointer)
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    Some(HistoryEvent {
        decision: text("/decision/decision")?,
        action_kind: text("/decision/action_kind"),
        experiment_id: text("/experiment_id"),
        action_id: text("/action_id"),
        situation: text("/situation").unwrap_or_default(),
        controller_id: text("/controller_id").unwrap_or_default(),
        mode: text("/mode").unwrap_or_default(),
        unix_nanos: value
            .pointer("/unix_nanos")
            .and_then(Value::as_u64)
            .map_or(0, u128::from),
        rollback_performed: value
            .pointer("/rollback_performed")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        reason: text("/reason").unwrap_or_default(),
    })
}

/// Follows an append-only history file from the point it was created at.
pub struct Tail {
    path: PathBuf,
    offset: u64,
}

impl Tail {
    pub fn at_end(path: &Path) -> Self {
        let offset = fs::metadata(path).map_or(0, |meta| meta.len());
        Self {
            path: path.to_owned(),
            offset,
        }
    }

    /// New complete lines since the last poll, as events.
    pub fn poll(&mut self) -> Vec<HistoryEvent> {
        let Ok(mut file) = fs::File::open(&self.path) else {
            return Vec::new();
        };
        let Ok(length) = file.metadata().map(|meta| meta.len()) else {
            return Vec::new();
        };
        if length < self.offset {
            self.offset = 0;
        }
        if file.seek(SeekFrom::Start(self.offset)).is_err() {
            return Vec::new();
        }
        let mut bytes = Vec::new();
        if file.read_to_end(&mut bytes).is_err() {
            return Vec::new();
        }
        // Only consume complete lines: a half-written record is read on the next poll.
        let complete = bytes
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |at| at + 1);
        self.offset += complete as u64;
        String::from_utf8_lossy(&bytes[..complete])
            .lines()
            .filter_map(parse_history_line)
            .collect()
    }
}

/// Every non-observed event of a finished history file, in order.
pub fn read_events(path: &Path) -> anyhow::Result<Vec<HistoryEvent>> {
    use std::io::BufRead;
    let file = fs::File::open(path)?;
    Ok(std::io::BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| parse_history_line(&line))
        .collect())
}

/// The first resolution recorded for each experiment, in order.
pub fn first_resolutions(events: &[HistoryEvent]) -> Vec<&HistoryEvent> {
    let mut seen = std::collections::BTreeSet::new();
    events
        .iter()
        .filter(|event| event.is_resolution())
        .filter(|event| seen.insert(event.experiment_id.clone()))
        .collect()
}

/// Audit events whose `command` is `command`, as (success, message) pairs.
pub fn audit_events(path: &Path, command: &str) -> Vec<(bool, String)> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|event| event["command"] == command)
        .map(|event| {
            (
                event["success"].as_bool().unwrap_or(false),
                event["message"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observed_records_are_skipped_and_decisions_parsed() {
        assert!(parse_history_line(r#"{"decision":{"decision":"observed"}}"#).is_none());
        let line = r#"{"unix_nanos":5,"controller_id":"local-autotune","mode":"ApplyMediumRisk","situation":"GameCpuSchedulerPressure","decision":{"decision":"candidate_applied","action_kind":"cgroup_placement"},"experiment_id":"e1","rollback_performed":false,"reason":"r"}"#;
        let event = parse_history_line(line).unwrap();
        assert_eq!(event.decision, "candidate_applied");
        assert_eq!(event.action_kind.as_deref(), Some("cgroup_placement"));
        assert_eq!(event.experiment_id.as_deref(), Some("e1"));
        assert!(!event.is_resolution());
    }

    #[test]
    fn the_tail_returns_only_new_complete_lines() {
        let dir = std::env::temp_dir().join(format!("soak-tail-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.jsonl");
        fs::write(&path, "{\"decision\":{\"decision\":\"candidate_kept\"}}\n").unwrap();
        let mut tail = Tail::at_end(&path);
        assert!(tail.poll().is_empty());
        let mut text = fs::read_to_string(&path).unwrap();
        text.push_str(
            "{\"decision\":{\"decision\":\"candidate_reverted\"}}\n{\"decision\":{\"decis",
        );
        fs::write(&path, text).unwrap();
        let events = tail.poll();
        assert_eq!(events.len(), 1);
        assert!(events[0].is_resolution());
        assert!(tail.poll().is_empty(), "the torn line is not consumed");
    }
}
