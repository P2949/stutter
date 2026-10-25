//! Durable study state: an immutable header, an append-only run log and an append-only decision
//! log. Progress is always recomputed from the logs, so a killed run resumes where it stopped.
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::validation_campaign::{scenarios::model::Family, state::write_atomic};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Control,
    Positive,
    Mixed,
}

/// One capture attempt, accepted or rejected.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RunRecord {
    pub seq: u64,
    pub scenario: String,
    pub family: Option<Family>,
    pub role: Role,
    /// Pair index for controls/positives, mixed capture index for mixed.
    pub pair: usize,
    pub accepted: bool,
    pub stage: Option<String>,
    pub reasons: Vec<String>,
    pub capture_id: Option<String>,
    pub fixture: Option<String>,
    /// `primary` (the family's direct metric) and supporting measurements.
    pub metrics: Value,
    pub unix_ms: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Pass,
    Extend,
    Invalid,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Decision {
    pub family: Family,
    pub pairs: usize,
    pub outcome: Outcome,
    /// Digest of the target-impact report that justified the decision.
    pub report_sha256: String,
    pub unix_ms: u64,
}

/// Facts frozen once so every capture and every resumed run uses the same ones.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Frozen {
    pub study_cpu: Option<u32>,
    pub irq: Option<u32>,
    pub irq_cpu: Option<u32>,
    pub fixed_work_ios: Option<u64>,
    /// CPUs the interrupt traffic senders are confined to (never the target's, the IRQ's or
    /// their SMT siblings).
    #[serde(default)]
    pub traffic_cpus: Vec<u32>,
}

/// Evidence taken over from an earlier study whose definition of these families is identical.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Import {
    pub from_study: String,
    pub from_base_commit: String,
    pub families: Vec<Family>,
    pub runs: usize,
    pub runs_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StudyHeader {
    pub schema_version: u32,
    pub study_id: String,
    pub base_commit: String,
    pub config_sha256: String,
    /// The core-validation campaign that only anchors the privileged runner's authorization.
    pub companion_campaign: String,
    pub frozen: Frozen,
    #[serde(default)]
    pub imports: Vec<Import>,
    pub created_unix_ms: u64,
}

pub struct StudyDir(pub PathBuf);

impl StudyDir {
    pub fn header_path(&self) -> PathBuf {
        self.0.join("study.json")
    }
    pub fn runs_path(&self) -> PathBuf {
        self.0.join("runs.ndjson")
    }
    pub fn decisions_path(&self) -> PathBuf {
        self.0.join("decisions.ndjson")
    }
    pub fn fixtures(&self) -> PathBuf {
        self.0.join("fixtures")
    }
    pub fn impact(&self) -> PathBuf {
        self.0.join("impact")
    }
    pub fn calibration(&self) -> PathBuf {
        self.0.join("calibration")
    }

    pub fn load_header(&self) -> anyhow::Result<StudyHeader> {
        let bytes = fs::read(self.header_path()).context("read study.json")?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    pub fn save_header(&self, header: &StudyHeader) -> anyhow::Result<()> {
        write_atomic(&self.header_path(), &serde_json::to_vec_pretty(header)?)
    }

    pub fn runs(&self) -> anyhow::Result<Vec<RunRecord>> {
        read_ndjson(&self.runs_path())
    }

    pub fn decisions(&self) -> anyhow::Result<Vec<Decision>> {
        read_ndjson(&self.decisions_path())
    }

    pub fn append_run(&self, record: &RunRecord) -> anyhow::Result<()> {
        append_ndjson(&self.runs_path(), record)
    }

    pub fn append_decision(&self, decision: &Decision) -> anyhow::Result<()> {
        append_ndjson(&self.decisions_path(), decision)
    }
}

pub fn read_ndjson<T: DeserializeOwned>(path: &Path) -> anyhow::Result<Vec<T>> {
    let Ok(text) = fs::read_to_string(path) else {
        return Ok(Vec::new());
    };
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).with_context(|| format!("parse {}", path.display())))
        .collect()
}

pub fn append_ndjson<T: Serialize>(path: &Path, record: &T) -> anyhow::Result<()> {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{}", serde_json::to_string(record)?)?;
    file.sync_all()?;
    Ok(())
}
