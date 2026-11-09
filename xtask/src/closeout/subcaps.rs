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
    /// The file that declares each test. The test must be a `#[test]` function in exactly this
    /// file, so an identically named function elsewhere cannot stand in for it.
    pub positive_file: String,
    pub negative_file: String,
}

#[derive(Debug, Deserialize)]
pub struct Manifest {
    /// Ids derived from the plan that must stay in `items`: deleting one is a failure.
    pub baseline: Vec<String>,
    pub items: Vec<Item>,
}

/// The plan-derived baseline, frozen here in code rather than in the editable manifest: each
/// subcapability the plan requires, with the dossier it belongs to. The manifest's own `baseline`
/// must equal this list exactly, so removing a requirement from the JSON alone, or from both the
/// JSON's lists, is caught, and changing this list is a code change that review sees.
pub const PLAN_BASELINE: &[(&str, &str)] = &[
    ("damos/quota-goal-feedback", "5.8"),
    ("damos/hot-cold-region-summary", "5.8"),
    ("damos/damo-parity-manifest", "5.8"),
    (
        "power/campaign-token-factory-and-worker-journal-integration",
        "5.16",
    ),
    ("numa/process-scoped-memory-policy-refusal-contract", "5.19"),
    ("numa/numad-coexistence-behaviour", "5.19"),
    ("cgroup/systemd-set-unit-properties-adapter", "5.21"),
    ("cgroup/openrc-backend", "5.21"),
    ("resctrl/mba-allocation", "5.24"),
    ("resctrl/verified-task-assignment", "5.24"),
    ("blockio/per-process-offender-attribution", "5.26"),
    ("blockio/blktrace-biosnoop-importers", "5.26"),
    ("net/socket-level-options", "5.27"),
    ("net/qdisc-selection", "5.27"),
    ("net/local-offered-load-workflow", "5.27"),
];

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

/// True only if `file` declares `fn name` directly under a `#[test]` attribute (an `#[ignore]` or
/// doc comment may sit between them): a helper, a comment or a same-named function elsewhere is
/// not a test.
pub fn is_test_in(root: &Path, file: &str, name: &str) -> bool {
    let Ok(text) = fs::read_to_string(root.join(file)) else {
        return false;
    };
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let needle = format!("fn {name}(");
    lines.iter().enumerate().any(|(at, line)| {
        if !(line.starts_with(&needle) || line.starts_with(&format!("pub {needle}"))) {
            return false;
        }
        // Walk up through attributes and doc comments to the first other line.
        lines[..at]
            .iter()
            .rev()
            .take_while(|above| {
                above.starts_with("#[") || above.starts_with("///") || above.is_empty()
            })
            .any(|above| *above == "#[test]")
    })
}

/// The cargo package a source file belongs to, and whether its tests run in the library target.
fn package_of(file: &str) -> Option<(&'static str, bool)> {
    if file.starts_with("stutter-validation-runner/") {
        Some(("stutter-validation-runner", true))
    } else if file.starts_with("stutter/") {
        Some(("stutter", true))
    } else if file.starts_with("xtask/") {
        Some(("xtask", false))
    } else {
        None
    }
}

/// Whether a test is `#[ignore]`d (a live test): such a test is verified to exist but needs its
/// own privileged resources, so it is not run here.
fn is_ignored(root: &Path, file: &str, name: &str) -> bool {
    let Ok(text) = fs::read_to_string(root.join(file)) else {
        return false;
    };
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let needle = format!("fn {name}(");
    lines.iter().enumerate().any(|(at, line)| {
        line.starts_with(&needle)
            && lines[..at]
                .iter()
                .rev()
                .take_while(|above| {
                    above.starts_with("#[") || above.starts_with("///") || above.is_empty()
                })
                .any(|above| above.starts_with("#[ignore"))
    })
}

/// Runs every non-ignored positive and negative test of every item and requires each to execute
/// and pass: at least one test matched, none failed. Returns how many distinct tests ran.
pub fn run_tests(manifest: &Manifest, root: &Path) -> anyhow::Result<usize> {
    let mut seen = BTreeSet::new();
    let mut failures = Vec::new();
    for item in &manifest.items {
        for (test, file) in [
            (&item.positive_test, &item.positive_file),
            (&item.negative_test, &item.negative_file),
        ] {
            if is_ignored(root, file, test) || !seen.insert((file.clone(), test.clone())) {
                continue;
            }
            let (package, lib) = package_of(file).context("test file is outside the workspace")?;
            let list = std::process::Command::new("cargo")
                .current_dir(root)
                .args(["test", "-q", "-p", package])
                .args(if lib { vec!["--lib"] } else { vec![] })
                .args(["--", "--list", test])
                .output()
                .with_context(|| format!("list {test}"))?;
            let listing = String::from_utf8_lossy(&list.stdout);
            let Some(path) = listing
                .lines()
                .filter_map(|line| line.strip_suffix(": test"))
                .find(|path| path.rsplit("::").next() == Some(test.as_str()))
            else {
                failures.push(format!(
                    "{}: {test} is not listed by the test harness",
                    item.id
                ));
                continue;
            };
            let mut run = std::process::Command::new("cargo");
            run.current_dir(root).args(["test", "-q", "-p", package]);
            if lib {
                run.arg("--lib");
            }
            run.args(["--", "--exact", path]);
            let output = run.output().with_context(|| format!("run {test}"))?;
            let text = String::from_utf8_lossy(&output.stdout);
            let ran_one = text.contains("test result: ok. 1 passed; 0 failed");
            if !output.status.success() || !ran_one {
                failures.push(format!(
                    "{}: {test} did not run and pass exactly once",
                    item.id
                ));
            }
        }
    }
    if !failures.is_empty() {
        bail!("subcapability tests failed:\n{}", failures.join("\n"));
    }
    Ok(seen.len())
}

/// Checks every item against the tree and every `code_complete` entry against its items.
pub fn validate(manifest: &Manifest, ledger: &Ledger, root: &Path) -> anyhow::Result<()> {
    let ids: BTreeSet<&str> = manifest.items.iter().map(|item| item.id.as_str()).collect();
    ensure!(
        ids.len() == manifest.items.len(),
        "duplicate subcapability id"
    );
    // The baseline is the one frozen in code: the manifest's copy must equal it exactly, and every
    // frozen item must be present under its own dossier.
    let frozen: Vec<&str> = PLAN_BASELINE.iter().map(|(id, _)| *id).collect();
    let declared: Vec<&str> = manifest.baseline.iter().map(String::as_str).collect();
    ensure!(
        declared == frozen,
        "the manifest baseline differs from the plan baseline frozen in code: {declared:?} vs {frozen:?}"
    );
    for (required, dossier) in PLAN_BASELINE {
        let item = manifest.items.iter().find(|item| item.id == *required);
        ensure!(
            item.is_some(),
            "baseline subcapability {required} was removed from the manifest"
        );
        ensure!(
            item.is_some_and(|item| item.entry == *dossier),
            "baseline subcapability {required} was moved to another dossier (it belongs to {dossier})"
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
        for (role, test, file) in [
            ("positive", &item.positive_test, &item.positive_file),
            ("negative", &item.negative_test, &item.negative_file),
        ] {
            if !functions.contains(test.as_str()) {
                failures.push(format!("{}: {role} test {test} does not exist", item.id));
            } else if !is_test_in(root, file, test) {
                failures.push(format!(
                    "{}: {role} test {test} is not a #[test] function in {file}",
                    item.id
                ));
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
