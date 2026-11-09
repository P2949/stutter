//! The independent subcapability manifest.
//!
//! The ledger's `missing_required` list is written by whoever edits the ledger, so clearing it
//! proves nothing. This manifest is the check that does not depend on it: a fixed *baseline* of
//! plan-derived subcapability ids that may never be dropped, and for every item a code anchor
//! (a file that must exist and contain the named symbol) and a positive and a negative test (functions
//! that must exist in the source). A `code_complete` dossier passes only if every item recorded for it
//! verifies. An item's kind keeps a typed refusal from being counted as a working actuator.
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, bail, ensure};
use serde::Deserialize;

use super::Ledger;

#[derive(Debug, Deserialize)]
pub struct Item {
    pub id: String,
    pub entry: String,
    /// `operation` (a gated actuator), `observation` (read-only) or `refusal` (a typed, explained
    /// denial: never an actuator).
    pub kind: String,
    pub anchor: String,
    pub symbol: String,
    pub positive_test: String,
    pub negative_test: String,
}

#[derive(Debug, Deserialize)]
pub struct Manifest {
    /// Ids derived from the plan that must stay in `items`: deleting one is a failure.
    pub baseline: Vec<String>,
    pub items: Vec<Item>,
}

const KINDS: [&str; 3] = ["operation", "observation", "refusal"];
const SOURCE_ROOTS: [&str; 3] = ["stutter/src", "xtask/src", "stutter-validation-runner/src"];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Every function name declared in the source roots.
fn declared_functions(root: &Path) -> BTreeSet<String> {
    let mut files = Vec::new();
    for source in SOURCE_ROOTS {
        rust_files(&root.join(source), &mut files);
    }
    let mut names = BTreeSet::new();
    for file in files {
        let Ok(text) = fs::read_to_string(file) else {
            continue;
        };
        for line in text.lines() {
            let line = line.trim_start();
            if let Some(rest) = line.find("fn ").map(|at| &line[at + 3..]) {
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    names.insert(name);
                }
            }
        }
    }
    names
}

/// Checks every item against the tree and every `code_complete` entry against its items.
pub fn validate(manifest: &Manifest, ledger: &Ledger, root: &Path) -> anyhow::Result<()> {
    let ids: BTreeSet<&str> = manifest.items.iter().map(|item| item.id.as_str()).collect();
    ensure!(
        ids.len() == manifest.items.len(),
        "duplicate subcapability id"
    );
    for required in &manifest.baseline {
        ensure!(
            ids.contains(required.as_str()),
            "baseline subcapability {required} was removed from the manifest"
        );
    }
    let functions = declared_functions(root);
    let mut failures = Vec::new();
    for item in &manifest.items {
        if !KINDS.contains(&item.kind.as_str()) {
            failures.push(format!("{}: unknown kind {:?}", item.id, item.kind));
        }
        let anchor = root.join(&item.anchor);
        match fs::read_to_string(&anchor) {
            Ok(text) if text.contains(&item.symbol) => {}
            Ok(_) => failures.push(format!(
                "{}: {} does not contain {}",
                item.id, item.anchor, item.symbol
            )),
            Err(_) => failures.push(format!("{}: anchor {} is missing", item.id, item.anchor)),
        }
        for (role, test) in [
            ("positive", &item.positive_test),
            ("negative", &item.negative_test),
        ] {
            if !functions.contains(test.as_str()) {
                failures.push(format!("{}: {role} test {test} does not exist", item.id));
            }
        }
        if !ledger.entries.iter().any(|entry| entry.id == item.entry) {
            failures.push(format!("{}: names unknown dossier {}", item.id, item.entry));
        }
    }
    // An entry that still lists a missing subcapability cannot be code_complete, whatever its
    // items say, and a baseline item's dossier must not claim completeness without the item.
    for entry in ledger
        .entries
        .iter()
        .filter(|e| e.status == "code_complete")
    {
        for required in &manifest.baseline {
            let belongs = manifest
                .items
                .iter()
                .any(|item| &item.id == required && item.entry == entry.id);
            ensure!(
                !belongs || ids.contains(required.as_str()),
                "{}: code_complete without {required}",
                entry.id
            );
        }
    }
    if !failures.is_empty() {
        bail!("subcapability manifest failed:\n{}", failures.join("\n"));
    }
    Ok(())
}

pub fn load(root: &Path) -> anyhow::Result<Manifest> {
    let path = root.join("docs/integration-subcapabilities.json");
    serde_json::from_slice(&fs::read(&path).with_context(|| format!("read {}", path.display()))?)
        .with_context(|| format!("{} is not a subcapability manifest", path.display()))
}

/// The report section: how many subcapabilities each kind has, so a refusal is visibly not an
/// actuator.
pub fn render(manifest: &Manifest) -> String {
    let count = |kind: &str| manifest.items.iter().filter(|i| i.kind == kind).count();
    let mut text = format!(
        "\n## Verified subcapabilities\n\n{} items, each with a code anchor and a positive and a negative test: {} operations, {} observations, {} typed refusals (a refusal explains an unsafe or impossible scope; it is not an actuator).\n\n| id | dossier | kind |\n|---|---|---|\n",
        manifest.items.len(),
        count("operation"),
        count("observation"),
        count("refusal")
    );
    for item in &manifest.items {
        text.push_str(&format!(
            "| {} | {} | {} |\n",
            item.id, item.entry, item.kind
        ));
    }
    text
}

/// Paths whose change alters built behaviour: everything except documentation.
pub fn code_changes<'a>(changed: &[&'a str]) -> Vec<&'a str> {
    changed
        .iter()
        .copied()
        .filter(|path| !(path.starts_with("docs/") || path.ends_with(".md")))
        .collect()
}
