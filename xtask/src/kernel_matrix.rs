//! Kernel compatibility matrix over verified `live_kernel_compat` evidence in
//! `docs/evidence/index.json`. Never infers compatibility from distribution or kernel name.
use std::{collections::BTreeMap, fs, path::Path};

use anyhow::{Context, ensure};
use serde_json::Value;

pub const STATES: [&str; 3] = [
    "ObservedCompatible",
    "ObservedDegraded",
    "ObservedIncompatible",
];

/// Latest observed state per distinct kernel release from PASS/FAIL-complete entries.
pub fn matrix(index: &Value) -> anyhow::Result<BTreeMap<String, String>> {
    ensure!(
        index["schema_version"] == 1,
        "unsupported evidence index schema"
    );
    let mut kernels = BTreeMap::new();
    for entry in index["entries"].as_array().context("entries")? {
        if entry["suite"] != "live_kernel_compat" || entry["status"] == "BLOCKED" {
            continue;
        }
        let kernel = entry["kernel_release"]
            .as_str()
            .context("kernel entry lacks kernel_release")?;
        let state = entry["compatibility"]
            .as_str()
            .context("kernel entry lacks compatibility")?;
        ensure!(
            STATES.contains(&state),
            "unknown compatibility state {state}"
        );
        kernels.insert(kernel.to_owned(), state.to_owned());
    }
    Ok(kernels)
}

pub fn run(root: &Path, require: usize) -> anyhow::Result<()> {
    let path = root.join("docs/evidence/index.json");
    let index: Value = serde_json::from_slice(&fs::read(&path)?)?;
    let kernels = matrix(&index)?;
    for (kernel, state) in &kernels {
        println!("{kernel}: {state}");
    }
    let usable = kernels
        .values()
        .filter(|state| *state != "ObservedIncompatible")
        .count();
    println!(
        "distinct kernels observed: {} ({usable} usable); Untested otherwise",
        kernels.len()
    );
    ensure!(
        kernels.len() >= require,
        "need evidence from at least {require} distinct real kernels, have {}",
        kernels.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_by_kernel_ignores_blocked_and_rejects_unknown_states() {
        let entry = |kernel: &str, state: &str, status: &str| serde_json::json!({"suite":"live_kernel_compat","status":status,"kernel_release":kernel,"compatibility":state});
        let index = serde_json::json!({"schema_version":1,"entries":[
            entry("6.18.55", "ObservedCompatible", "PASS"),
            entry("6.12.1-zen", "ObservedDegraded", "PASS"),
            entry("6.1.0", "ObservedIncompatible", "FAIL"),
            entry("5.15.0", "Untested", "BLOCKED"),
            {"suite":"live_affinity","status":"PASS"},
        ]});
        let kernels = matrix(&index).unwrap();
        assert_eq!(kernels.len(), 4 - 1);
        assert_eq!(kernels["6.12.1-zen"], "ObservedDegraded");
        let bad = serde_json::json!({"schema_version":1,"entries":[entry("x", "Guess", "PASS")]});
        assert!(matrix(&bad).is_err());
    }
}
