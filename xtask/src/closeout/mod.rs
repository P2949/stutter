//! The local closeout report (package W19): every package and tool with its truthful disposition.
//!
//! The report is generated from `docs/integration-coverage.json` and refuses to overstate it. A
//! disposition outside the vocabulary, a `CodeComplete` on an entry whose status is not
//! `code_complete`, a physical-proof disposition without evidence, or an entry with no disposition
//! at all fails generation, so the report cannot say more than the ledger can show.
#[cfg(test)]
mod tests;

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, bail, ensure};
use clap::Args;
use serde::Deserialize;

/// The dispositions a closeout may use, weakest to strongest.
pub const VOCABULARY: [&str; 9] = [
    "EvidenceDeferred",
    "HardwareUnavailable",
    "SafetyAborted",
    "FixtureVerified",
    "RealObserved",
    "CodeComplete",
    "MechanismValidated",
    "BenefitValidated",
    "AutonomySupported",
];

/// Dispositions that claim physical proof and so need an evidence path.
const NEEDS_EVIDENCE: [&str; 3] = [
    "MechanismValidated",
    "BenefitValidated",
    "AutonomySupported",
];

#[derive(Debug, Deserialize)]
pub struct Entry {
    pub id: String,
    pub tools: Vec<String>,
    pub packages: Vec<String>,
    pub status: String,
    pub dispositions: Vec<String>,
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default)]
    pub unblock: Option<String>,
    pub g0: String,
    pub g1: String,
    pub not_wired: String,
}

#[derive(Debug, Deserialize)]
pub struct Ledger {
    pub source_sha: String,
    pub entries: Vec<Entry>,
}

pub fn validate(ledger: &Ledger) -> anyhow::Result<()> {
    for entry in &ledger.entries {
        let id = &entry.id;
        ensure!(!entry.dispositions.is_empty(), "{id}: no disposition");
        for disposition in &entry.dispositions {
            ensure!(
                VOCABULARY.contains(&disposition.as_str()),
                "{id}: {disposition:?} is not a disposition"
            );
            if disposition == "CodeComplete" {
                ensure!(
                    entry.status == "code_complete",
                    "{id}: CodeComplete but the status is {}",
                    entry.status
                );
            }
            if NEEDS_EVIDENCE.contains(&disposition.as_str()) {
                ensure!(
                    !entry.evidence.is_empty(),
                    "{id}: {disposition} without an evidence path"
                );
            }
        }
        if entry.status == "code_complete" {
            ensure!(
                entry.dispositions.iter().any(|d| d == "FixtureVerified"),
                "{id}: code_complete needs verified fixtures"
            );
        }
        if entry
            .dispositions
            .iter()
            .any(|d| d == "HardwareUnavailable")
        {
            ensure!(
                entry.unblock.is_some(),
                "{id}: HardwareUnavailable without an unblock condition"
            );
        }
    }
    Ok(())
}

/// The report as Markdown, for a validated ledger.
pub fn render(ledger: &Ledger, kernel: &str) -> String {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for entry in &ledger.entries {
        for disposition in &entry.dispositions {
            *counts.entry(disposition.as_str()).or_default() += 1;
        }
    }
    let mut text = format!(
        "# Closeout report\n\nSource `{}`, kernel `{kernel}`. Generated from the integration coverage ledger; \
it cannot claim more than the ledger records.\n\n## Disposition counts\n\n| disposition | entries |\n|---|---|\n",
        ledger.source_sha
    );
    for name in VOCABULARY {
        text.push_str(&format!(
            "| {name} | {} |\n",
            counts.get(name).copied().unwrap_or(0)
        ));
    }
    text.push_str("\n## Entries\n\n| id | tools | packages | status | dispositions | not wired |\n|---|---|---|---|---|---|\n");
    for entry in &ledger.entries {
        text.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            entry.id,
            entry.tools.join(", "),
            entry.packages.join(" "),
            entry.status,
            entry.dispositions.join(", "),
            entry.not_wired.replace('|', "/")
        ));
    }
    text.push_str("\n## Unavailable physical proof and what unblocks it\n\n");
    for entry in ledger.entries.iter().filter(|e| e.unblock.is_some()) {
        text.push_str(&format!(
            "* **{}** ({}): {}\n",
            entry.id,
            entry.tools.join(", "),
            entry.unblock.as_deref().unwrap_or("")
        ));
    }
    text
}

#[derive(Args, Clone, Debug, Eq, PartialEq)]
pub struct CloseoutArgs {
    /// Where to write the report; printed when omitted.
    #[arg(long)]
    pub out: Option<PathBuf>,
}

pub fn run(root: &Path, args: &CloseoutArgs) -> anyhow::Result<()> {
    let path = root.join("docs/integration-coverage.json");
    let ledger: Ledger = serde_json::from_slice(&fs::read(&path)?)
        .with_context(|| format!("{} is not a coverage ledger", path.display()))?;
    validate(&ledger)?;
    let head = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()?;
    let head = String::from_utf8_lossy(&head.stdout).trim().to_owned();
    if head != ledger.source_sha {
        // The ledger names the source it describes; a report from another HEAD says so.
        eprintln!(
            "note: the ledger describes {} and HEAD is {head}",
            ledger.source_sha
        );
    }
    // How far HEAD has moved past the source the ledger describes: a stale record says so.
    let behind = Command::new("git")
        .args([
            "rev-list",
            "--count",
            &format!("{}..HEAD", ledger.source_sha),
        ])
        .current_dir(root)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned());
    match behind {
        Some(count) if count != "0" => {
            eprintln!(
                "note: the ledger is {count} commit(s) behind HEAD; re-verify before relying on it"
            );
        }
        Some(_) => {}
        None => eprintln!(
            "note: the ledger's source {} is not in this history",
            ledger.source_sha
        ),
    }
    let kernel = fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
    let report = render(&ledger, kernel.trim());
    match &args.out {
        Some(out) => fs::write(out, &report).with_context(|| format!("write {}", out.display()))?,
        None => print!("{report}"),
    }
    if ledger.entries.iter().any(|e| e.status != "code_complete") {
        bail!(
            "{} of {} entries are not code_complete: the closeout gate is not met",
            ledger
                .entries
                .iter()
                .filter(|e| e.status != "code_complete")
                .count(),
            ledger.entries.len()
        );
    }
    Ok(())
}
