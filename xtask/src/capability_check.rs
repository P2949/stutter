//! A capability may be `Supported` only if every live suite it requires has a PASS entry of
//! kind `controlled_real` or `real_workload` in the verified-evidence index. Also reports graduation gaps.
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
        // A live-suite requirement is met only by real-system evidence: a synthetic or
        // fixture-real PASS that merely carries the suite name does not count.
        .filter(|entry| {
            matches!(
                entry["evidence_kind"].as_str(),
                Some("controlled_real" | "real_workload")
            )
        })
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
        let (violations, _) = evaluate(&indexed_passing_suites(&index).unwrap());
        assert!(violations.is_empty(), "{violations:?}");
    }

    #[test]
    fn a_supported_capability_without_indexed_evidence_is_a_violation() {
        // Simulate a registry decision made ahead of evidence by checking the rule directly.
        let (violations, _) = evaluate(&BTreeSet::new());
        let supported = capability_registry()
            .iter()
            .filter(|c| c.tier == CapabilityTier::Supported && !c.required_evidence.is_empty())
            .count();
        assert_eq!(violations.len(), supported, "{violations:?}");
    }

    #[test]
    fn only_real_live_kinds_satisfy_a_suite_requirement() {
        let entry = |kind: &str, suite: &str| serde_json::json!({"suite":suite,"status":"PASS","evidence_kind":kind});
        let index = serde_json::json!({"schema_version":1,"entries":[
            entry("synthetic", "live_observation"),
            entry("fixture_real", "live_recovery"),
            entry("controlled_real", "live_affinity"),
            entry("real_workload", "live_overhead"),
            {"suite":"live_kernel_compat","status":"FAIL","evidence_kind":"controlled_real"},
            {"suite":"live_game_runnable","status":"PASS"},
        ]});
        let suites = indexed_passing_suites(&index).unwrap();
        assert_eq!(
            suites.into_iter().collect::<Vec<_>>(),
            ["live_affinity", "live_overhead"]
        );
    }

    #[test]
    fn the_bounded_controller_cannot_be_supported_without_the_indexed_soak() {
        use stutter::api::capabilities::capability_registry;
        let controller = capability_registry()
            .iter()
            .find(|capability| capability.id == "bounded_autotune_controller")
            .unwrap();
        let everything_but_the_soak = controller
            .required_evidence
            .iter()
            .filter(|suite| **suite != "live_autotune_soak")
            .map(|suite| (*suite).to_owned())
            .collect::<BTreeSet<_>>();
        // The controller is Supported, so missing exactly the soak is a tier violation naming it...
        assert_eq!(controller.tier, CapabilityTier::Supported);
        let (violations, _) = evaluate(&everything_but_the_soak);
        assert!(
            violations
                .iter()
                .any(|v| v.contains("bounded_autotune_controller")
                    && v.contains("live_autotune_soak")),
            "{violations:?}"
        );
        // ...the smoke suite is not the graduation proof...
        let only_smoke = BTreeSet::from(["live_autotune_smoke".to_owned()]);
        assert!(
            evaluate(&only_smoke)
                .0
                .iter()
                .any(|v| v.contains("bounded_autotune_controller"))
        );
        // ...and with every required suite indexed, including the soak, there is no violation.
        let complete = controller
            .required_evidence
            .iter()
            .map(|suite| (*suite).to_owned())
            .collect::<BTreeSet<_>>();
        assert!(
            !evaluate(&complete)
                .0
                .iter()
                .any(|v| v.contains("bounded_autotune_controller"))
        );
    }
}
