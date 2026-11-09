//! The local closeout report (package W19): every package and tool with its truthful disposition.
//!
//! The report is generated from `docs/integration-coverage.json` and refuses to overstate it. A
//! disposition outside the vocabulary, a `CodeComplete` on an entry whose status is not
//! `code_complete`, a physical-proof disposition without evidence, or an entry with no disposition
//! at all fails generation, so the report cannot say more than the ledger can show.
mod subcaps;
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
    /// Stable ids of plan-required subcapabilities with no implementation yet. Distinct from
    /// `not_wired`, which also records physical evidence that cannot be gathered here: a
    /// `code_complete` entry has none of these.
    #[serde(default)]
    pub missing_required: Vec<String>,
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
            // The status and the disposition say the same thing: an entry whose implementation is
            // complete carries the `CodeComplete` disposition, and only such an entry does.
            ensure!(
                entry.dispositions.iter().any(|d| d == "CodeComplete"),
                "{id}: status code_complete but no CodeComplete disposition"
            );
            ensure!(
                entry.missing_required.is_empty(),
                "{id}: code_complete but required subcapabilities are missing: {}",
                entry.missing_required.join(", ")
            );
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
    /// Final qualification: the tree must be clean and the ledger's source must differ from HEAD
    /// only by documentation, so the ledger describes the code that is actually being qualified.
    #[arg(long)]
    pub strict: bool,
    /// Execute every non-ignored positive and negative test of every subcapability and require
    /// each to run and pass (implied by `--strict`).
    #[arg(long)]
    pub run_tests: bool,
}

pub fn run(root: &Path, args: &CloseoutArgs) -> anyhow::Result<()> {
    let path = root.join("docs/integration-coverage.json");
    let ledger: Ledger = serde_json::from_slice(&fs::read(&path)?)
        .with_context(|| format!("{} is not a coverage ledger", path.display()))?;
    validate(&ledger)?;
    let manifest = subcaps::load(root)?;
    subcaps::validate(&manifest, &ledger, root)?;
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
    let mut report = render(&ledger, kernel.trim());
    report.push_str(&subcaps::render(&manifest));
    if args.strict {
        strict_source_check(root, &ledger.source_sha)?;
    }
    if args.strict || args.run_tests {
        let ran = subcaps::run_tests(&manifest, root)?;
        eprintln!("note: {ran} subcapability tests were executed and passed");
    }
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

/// Strict mode: the working tree is clean and nothing but documentation changed since the source
/// the ledger describes, so a receipt for that source qualifies the code at HEAD.
fn strict_source_check(root: &Path, source_sha: &str) -> anyhow::Result<()> {
    let git = |args: &[&str]| -> anyhow::Result<String> {
        let output = Command::new("git").args(args).current_dir(root).output()?;
        ensure!(output.status.success(), "git {args:?} failed");
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    };
    ensure!(
        git(&["status", "--porcelain"])?.trim().is_empty(),
        "strict closeout needs a clean working tree"
    );
    ensure!(
        Command::new("git")
            .args(["merge-base", "--is-ancestor", source_sha, "HEAD"])
            .current_dir(root)
            .status()?
            .success(),
        "the ledger's source {source_sha} is not an ancestor of HEAD"
    );
    let range = format!("{source_sha}..HEAD");
    let changed = git(&["diff", "--name-only", &range])?;
    let paths: Vec<&str> = changed.lines().collect();
    let code = subcaps::code_changes(&paths);
    ensure!(
        code.is_empty(),
        "code changed since the ledger's source {source_sha}: {}; re-verify and re-pin the ledger",
        code.join(", ")
    );
    Ok(())
}
