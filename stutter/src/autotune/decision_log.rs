use std::{
    fs::{self, OpenOptions},
    io::{BufRead, Write},
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ControllerPhaseLabel {
    Disabled,
    Observing,
    Planning,
    Applying,
    Measuring,
    Keeping,
    Reverting,
    Cooldown,
    Faulted,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum AutotuneModeLabel {
    Observe,
    Suggest,
    ApplyLowRisk,
    ApplyMediumRisk,
    ApplyHighRisk,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum SituationKindLabel {
    Unknown,
    Idle,
    GameFocused,
    GameCpuSchedulerPressure,
    GameGpuBound,
    CompositorPressure,
    CpuPressure,
    IoPressure,
    IrqPressure,
    ThermalOrPowerLimit,
    CompileLoad,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum OnlineDataQualityLabel {
    High,
    Medium,
    Low,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum AutotuneDecisionLabel {
    Noop,
    Suggest,
    StartExperiment,
    KeepCurrent,
    Revert,
    EnterCooldown,
    Fault,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DecisionJsonlEntry {
    pub schema_version: u32,
    pub unix_nanos: u128,
    pub phase: ControllerPhaseLabel,
    pub mode: AutotuneModeLabel,
    pub target_present: bool,
    pub situation: SituationKindLabel,
    pub score_total: u64,
    pub data_quality: OnlineDataQualityLabel,
    pub decision: AutotuneDecisionLabel,
    pub reason: String,
}

impl DecisionJsonlEntry {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        phase: ControllerPhaseLabel,
        mode: AutotuneModeLabel,
        target_present: bool,
        situation: SituationKindLabel,
        score_total: u64,
        data_quality: OnlineDataQualityLabel,
        decision: AutotuneDecisionLabel,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: 1,
            unix_nanos: crate::audit::unix_nanos_now(),
            phase,
            mode,
            target_present,
            situation,
            score_total,
            data_quality,
            decision,
            reason: reason.into(),
        }
    }

    pub fn observe_noop(
        target_present: bool,
        situation: SituationKindLabel,
        score_total: u64,
        data_quality: OnlineDataQualityLabel,
        reason: impl Into<String>,
    ) -> Self {
        Self::new(
            ControllerPhaseLabel::Observing,
            AutotuneModeLabel::Observe,
            target_present,
            situation,
            score_total,
            data_quality,
            AutotuneDecisionLabel::Noop,
            reason,
        )
    }
}

pub fn append_decision_jsonl(path: &Path, entry: &DecisionJsonlEntry) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create decision log directory {}",
                parent.display()
            )
        })?;
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open decision log {}", path.display()))?;

    serde_json::to_writer(&mut file, entry)
        .with_context(|| format!("failed to write decision log entry {}", path.display()))?;
    file.write_all(b"\n")
        .with_context(|| format!("failed to terminate decision log entry {}", path.display()))?;

    Ok(())
}

pub fn read_decision_jsonl(path: &Path) -> anyhow::Result<Vec<DecisionJsonlEntry>> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let file = fs::File::open(path)
        .with_context(|| format!("failed to open decision log {}", path.display()))?;
    let reader = std::io::BufReader::new(file);
    let mut entries = Vec::new();

    for line in reader.lines() {
        let line =
            line.with_context(|| format!("failed to read decision log {}", path.display()))?;
        if line.trim().is_empty() {
            continue;
        }
        let entry = serde_json::from_str::<DecisionJsonlEntry>(&line)
            .with_context(|| format!("failed to parse decision log {}", path.display()))?;
        entries.push(entry);
    }

    Ok(entries)
}

pub fn default_decision_log_path() -> PathBuf {
    let mut path = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    path.push(".local");
    path.push("state");
    path.push("stutter");
    path.push("autotune");
    path.push("decisions.jsonl");
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-autotune-decision-log-test-{name}-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn decision_entry_serializes_with_requested_schema_fields() {
        let mut entry = DecisionJsonlEntry::observe_noop(
            true,
            SituationKindLabel::GameCpuSchedulerPressure,
            143,
            OnlineDataQualityLabel::High,
            "observe mode; scheduler pressure detected but apply disabled",
        );
        entry.unix_nanos = 123456;

        let json = serde_json::to_string(&entry).unwrap();

        assert!(json.contains("\"schema_version\":1"));
        assert!(json.contains("\"unix_nanos\":123456"));
        assert!(json.contains("\"phase\":\"Observing\""));
        assert!(json.contains("\"mode\":\"Observe\""));
        assert!(json.contains("\"target_present\":true"));
        assert!(json.contains("\"situation\":\"GameCpuSchedulerPressure\""));
        assert!(json.contains("\"score_total\":143"));
        assert!(json.contains("\"data_quality\":\"High\""));
        assert!(json.contains("\"decision\":\"Noop\""));
        assert!(json.contains(
            "\"reason\":\"observe mode; scheduler pressure detected but apply disabled\""
        ));

        let parsed: DecisionJsonlEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, entry);
    }

    #[test]
    fn append_decision_jsonl_writes_one_json_object_per_line() {
        let dir = temp_dir("append");
        let path = dir.join("decisions.jsonl");

        let mut first = DecisionJsonlEntry::observe_noop(
            true,
            SituationKindLabel::GameCpuSchedulerPressure,
            143,
            OnlineDataQualityLabel::High,
            "observe mode; scheduler pressure detected but apply disabled",
        );
        first.unix_nanos = 123456;

        let mut second = DecisionJsonlEntry::new(
            ControllerPhaseLabel::Observing,
            AutotuneModeLabel::Suggest,
            true,
            SituationKindLabel::CpuPressure,
            200,
            OnlineDataQualityLabel::Medium,
            AutotuneDecisionLabel::Suggest,
            "suggest mode; candidate would be reported but not applied",
        );
        second.unix_nanos = 123457;

        append_decision_jsonl(&path, &first).unwrap();
        append_decision_jsonl(&path, &second).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        let lines = text.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 2);

        let parsed_first: DecisionJsonlEntry = serde_json::from_str(lines[0]).unwrap();
        let parsed_second: DecisionJsonlEntry = serde_json::from_str(lines[1]).unwrap();

        assert_eq!(parsed_first, first);
        assert_eq!(parsed_second, second);

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn read_decision_jsonl_returns_all_entries() {
        let dir = temp_dir("read");
        let path = dir.join("decisions.jsonl");

        let mut entry = DecisionJsonlEntry::observe_noop(
            true,
            SituationKindLabel::GameCpuSchedulerPressure,
            143,
            OnlineDataQualityLabel::High,
            "observe mode; scheduler pressure detected but apply disabled",
        );
        entry.unix_nanos = 123456;

        append_decision_jsonl(&path, &entry).unwrap();

        let entries = read_decision_jsonl(&path).unwrap();
        assert_eq!(entries, vec![entry]);

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn missing_decision_log_is_not_an_error() {
        let dir = temp_dir("missing");
        let path = dir.join("missing.jsonl");

        let entries = read_decision_jsonl(&path).unwrap();

        assert!(entries.is_empty());
        fs::remove_dir_all(dir).ok();
    }
}
