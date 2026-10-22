//! The committed campaign manifest (`docs/validation/core-validation.toml`): parameters of the
//! campaign, not its dependency graph (the DAG is encoded in `graph.rs`). Its SHA-256 is part of
//! every task fingerprint, so editing it invalidates source-bound results.
use std::{collections::BTreeSet, fs, path::Path};

use anyhow::{Context, bail, ensure};
use serde::{Deserialize, Serialize};

use crate::evidence_bundle::sha256::sha256_hex;

pub const MANIFEST_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReleaseSection {
    pub version: String,
    pub tag: String,
    pub auto_tag: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KernelSection {
    pub required_distinct: usize,
    /// Reboot budget is `min(candidates + reboot_slack, max_reboots)`.
    pub reboot_slack: usize,
    pub max_reboots: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CorpusSection {
    pub min_positive: usize,
    pub min_negative_control: usize,
    pub min_mixed: usize,
    /// Failed attempts allowed per required successful capture before `BlockedResource`.
    pub failed_attempts_per_capture: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OverheadSection {
    pub rounds: u32,
    pub seconds: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ScoreSection {
    pub levels_ms: Vec<u32>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CapabilityPolicy {
    pub supported: Vec<String>,
    pub experimental: Vec<String>,
    pub internal: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PolicySection {
    pub capabilities: CapabilityPolicy,
    /// Tasks explicitly not required. Only tasks the graph marks optional may appear.
    #[serde(default)]
    pub skip_tasks: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RetrySection {
    /// Attempts per task before a retryable failure becomes a failure.
    pub default_attempts: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CampaignManifest {
    pub schema_version: u32,
    pub name: String,
    pub release: ReleaseSection,
    pub kernels: KernelSection,
    pub corpus: CorpusSection,
    pub overhead: OverheadSection,
    pub score: ScoreSection,
    pub policy: PolicySection,
    pub retry: RetrySection,
}

impl CampaignManifest {
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let manifest: Self = toml::from_str(text).context("campaign manifest is not valid")?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Reads and validates `path`, returning the manifest with the digest of its exact bytes.
    pub fn load(path: &Path) -> anyhow::Result<(Self, String)> {
        let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let text = std::str::from_utf8(&bytes).context("manifest is not UTF-8")?;
        Ok((Self::parse(text)?, sha256_hex(&bytes)))
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        ensure!(
            self.schema_version == MANIFEST_SCHEMA_VERSION,
            "unsupported manifest schema {}",
            self.schema_version
        );
        ensure!(!self.name.trim().is_empty(), "campaign name is empty");
        let version = &self.release.version;
        ensure!(
            !version.is_empty()
                && version.split('.').count() == 3
                && version
                    .split('.')
                    .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit())),
            "release.version {version:?} must be MAJOR.MINOR.PATCH"
        );
        ensure!(
            self.release.tag == format!("v{version}"),
            "release.tag {:?} must be v{version}",
            self.release.tag
        );
        ensure!(
            self.kernels.required_distinct >= 1 && self.kernels.max_reboots >= 1,
            "kernel requirements must be positive"
        );
        ensure!(
            self.corpus.min_positive > 0
                && self.corpus.min_negative_control > 0
                && self.corpus.min_mixed > 0
                && self.corpus.failed_attempts_per_capture > 0,
            "corpus floors and the retry budget must be positive"
        );
        ensure!(
            self.overhead.rounds >= 6 && (10..=60).contains(&self.overhead.seconds),
            "overhead rounds must be >= 6 and seconds within 10..=60"
        );
        let levels = &self.score.levels_ms;
        ensure!(
            levels.first() == Some(&0)
                && levels.len() >= 4
                && levels.windows(2).all(|pair| pair[0] < pair[1]),
            "score levels must start at 0 and strictly increase (at least four levels)"
        );
        ensure!(
            self.retry.default_attempts >= 1,
            "retry attempts must be >= 1"
        );
        let capabilities = &self.policy.capabilities;
        let mut seen = BTreeSet::new();
        for id in capabilities
            .supported
            .iter()
            .chain(&capabilities.experimental)
            .chain(&capabilities.internal)
        {
            if !seen.insert(id.as_str()) {
                bail!("capability {id} appears in more than one policy tier");
            }
        }
        Ok(())
    }
}
