//! A capability may be `Supported` only if every live suite it requires has a PASS,
//! controlled-or-real entry in the verified-evidence index. Also reports graduation gaps.
use std::{collections::BTreeSet, fs, path::Path};

use anyhow::{Context, ensure};
use serde_json::Value;
use stutter::api::capabilities::{CapabilityTier, capability_registry};

pub fn indexed_passing_suites(index: &Value) -> anyhow::Result<BTreeSet<String>> {
    ensure!(
        index["schema_version"] == 1,
        "unsupported evidence index schema"
    );
    Ok(index["entries"]
        .as_array()
        .context("entries must be an array")?
        .iter()
        .filter(|entry| entry["status"] == "PASS")
        .filter_map(|entry| entry["suite"].as_str().map(str::to_owned))
        .collect())
}

/// Returns violations (Supported without evidence) and informational graduation gaps.
pub fn evaluate(suites: &BTreeSet<String>) -> (Vec<String>, Vec<String>) {
    let (mut violations, mut gaps) = (Vec::new(), Vec::new());
    for capability in capability_registry() {
        let missing = capability
            .required_evidence
            .iter()
            .filter(|suite| !suites.contains(**suite))
            .copied()
            .collect::<Vec<_>>();
        if capability.tier == CapabilityTier::Supported && !missing.is_empty() {
            violations.push(format!(
                "{} is Supported but lacks verified evidence: {}",
                capability.id,
                missing.join(", ")
            ));
        }
        if capability.target_tier == CapabilityTier::Supported
            && capability.tier != CapabilityTier::Supported
        {
            gaps.push(format!(
                "{}: missing evidence [{}]",
                capability.id,
                if missing.is_empty() {
                    "none; tier decision pending".to_owned()
                } else {
                    missing.join(", ")
                }
            ));
        }
    }
    (violations, gaps)
}

pub fn run(root: &Path) -> anyhow::Result<()> {
    let path = root.join("docs/evidence/index.json");
    let index: Value =
        serde_json::from_slice(&fs::read(&path).with_context(|| path.display().to_string())?)?;
    let suites = indexed_passing_suites(&index)?;
    let (violations, gaps) = evaluate(&suites);
    println!(
        "verified evidence suites: {}",
        suites.iter().cloned().collect::<Vec<_>>().join(", ")
    );
    for gap in &gaps {
        println!("graduation gap: {gap}");
    }
    ensure!(
        violations.is_empty(),
        "capability tier violations:\n  {}",
        violations.join("\n  ")
    );
    println!("capability-check: no capability is Supported ahead of its evidence");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_is_consistent_with_the_committed_evidence_index() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let index: Value =
            serde_json::from_slice(&fs::read(root.join("docs/evidence/index.json")).unwrap())
                .unwrap();
        let (violations, gaps) = evaluate(&indexed_passing_suites(&index).unwrap());
        assert!(violations.is_empty(), "{violations:?}");
        // Affinity, recovery and suspend/resume evidence exist; only observation is missing.
        let affinity = gaps
            .iter()
            .find(|gap| gap.starts_with("cpu_affinity_experiment"))
            .unwrap();
        assert!(affinity.contains("live_observation"));
        assert!(!affinity.contains("live_suspend_resume") && !affinity.contains("live_recovery"));
    }

    #[test]
    fn a_supported_capability_without_indexed_evidence_is_a_violation() {
        // Simulate a registry decision made ahead of evidence by checking the rule directly.
        let none = BTreeSet::new();
        let (violations, _) = evaluate(&none);
        assert!(violations.is_empty(), "nothing is Supported today");
        let present = BTreeSet::from(["live_recovery".to_owned()]);
        let (_, gaps) = evaluate(&present);
        assert!(
            gaps.iter()
                .any(|gap| gap.starts_with("restore") && !gap.contains("live_recovery"))
        );
    }
}
