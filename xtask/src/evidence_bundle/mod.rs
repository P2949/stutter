//! Canonical portable evidence bundles: export from raw live-acceptance output and
//! verify independently. Extends (does not replace) the live-acceptance harness.
//!
//! ```text
//! bundle/
//!   receipt.json  environment.json  validation.json  MANIFEST.sha256  README.md
//!   logs/         artifacts/
//! ```
use std::{
    fs,
    path::{Component, Path},
};

use anyhow::{Context, bail, ensure};
use serde_json::{Value, json};

mod export;
pub(crate) mod privacy;
pub(crate) mod sha256;
mod verify;

pub use export::export_bundle;
pub use verify::verify_bundle;

pub(crate) const BUNDLE_SCHEMA_VERSION: u64 = 1;
pub(crate) const TOP_LEVEL_FILES: [&str; 5] = [
    "receipt.json",
    "environment.json",
    "validation.json",
    "MANIFEST.sha256",
    "README.md",
];
pub(crate) const TOP_LEVEL_DIRS: [&str; 2] = ["logs", "artifacts"];
pub(crate) const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// Provenance class of the evidence; never inferred from file names.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EvidenceKind {
    Synthetic,
    FixtureReal,
    ControlledReal,
    RealWorkload,
}
impl EvidenceKind {
    pub(crate) const ALL: [Self; 4] = [
        Self::Synthetic,
        Self::FixtureReal,
        Self::ControlledReal,
        Self::RealWorkload,
    ];
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Synthetic => "synthetic",
            Self::FixtureReal => "fixture_real",
            Self::ControlledReal => "controlled_real",
            Self::RealWorkload => "real_workload",
        }
    }
    pub(crate) fn parse(value: &str) -> anyhow::Result<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.as_str() == value)
            .with_context(|| format!("unknown evidence_kind {value:?}"))
    }
}

pub(crate) const STATUSES: [&str; 4] = ["PASS", "FAIL", "BLOCKED", "INCONCLUSIVE"];
pub(crate) const KNOWN_SUITES: [&str; 5] = [
    "live_observation",
    "live_affinity",
    "live_recovery",
    "live_suspend_resume",
    "live_game_runnable",
];

/// Environment fields copied to the portable bundle. `proc_version` is excluded on
/// purpose: it embeds the kernel builder's user and host.
pub(crate) const ENVIRONMENT_FIELDS: [&str; 20] = [
    "schema_version",
    "suite",
    "evidence_kind",
    "capture_id",
    "captured_unix_ms",
    "commit",
    "source_dirty",
    "source_matches_binary",
    "source_matches_harness",
    "binary_version",
    "binary_sha256",
    "harness_sha256",
    "cargo_lock_sha256",
    "kernel_release",
    "distribution",
    "cpu_model",
    "cpu_online",
    "rust",
    "artifact_schema",
    "gpu_relevant",
];
/// Fields a bundle cannot be produced without.
pub(crate) const REQUIRED_ENVIRONMENT_FIELDS: [&str; 15] = [
    "schema_version",
    "suite",
    "evidence_kind",
    "capture_id",
    "captured_unix_ms",
    "commit",
    "source_dirty",
    "source_matches_binary",
    "binary_version",
    "binary_sha256",
    "cargo_lock_sha256",
    "kernel_release",
    "distribution",
    "cpu_model",
    "artifact_schema",
];

pub(crate) fn is_hex(value: &str, len: usize) -> bool {
    value.len() == len && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// A bundle path must stay inside the bundle and be relative.
pub(crate) fn ensure_relative(path: &str) -> anyhow::Result<()> {
    ensure!(!path.is_empty(), "empty path reference");
    let parsed = Path::new(path);
    ensure!(!parsed.is_absolute(), "absolute path reference {path}");
    ensure!(
        parsed
            .components()
            .all(|component| matches!(component, Component::Normal(_))),
        "non-normal path reference {path}"
    );
    Ok(())
}

pub(crate) fn read_json(path: &Path) -> anyhow::Result<Value> {
    serde_json::from_slice(&fs::read(path).with_context(|| format!("read {}", path.display()))?)
        .with_context(|| format!("invalid JSON in {}", path.display()))
}

pub(crate) fn field<'a>(value: &'a Value, name: &str) -> anyhow::Result<&'a Value> {
    value
        .get(name)
        .filter(|field| !field.is_null())
        .with_context(|| format!("missing field {name}"))
}

pub(crate) fn string_field<'a>(value: &'a Value, name: &str) -> anyhow::Result<&'a str> {
    field(value, name)?
        .as_str()
        .with_context(|| format!("field {name} is not a string"))
}

pub(crate) fn manifest_line(hash: &str, relative: &str) -> String {
    format!("{hash}  {relative}\n")
}

pub(crate) fn summary_json(checks: &[(String, bool, String)]) -> Value {
    json!(
        checks
            .iter()
            .map(|(name, passed, detail)| json!({"name":name,"passed":passed,"detail":detail}))
            .collect::<Vec<_>>()
    )
}

pub(crate) fn fail_if_any(checks: &[(String, bool, String)]) -> anyhow::Result<()> {
    let failed = checks
        .iter()
        .filter(|(_, passed, _)| !passed)
        .map(|(name, _, detail)| format!("{name}: {detail}"))
        .collect::<Vec<_>>();
    if !failed.is_empty() {
        bail!("evidence verification failed:\n  {}", failed.join("\n  "));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
