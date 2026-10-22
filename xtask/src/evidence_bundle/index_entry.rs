//! Builds the `docs/evidence/index.json` entry for a verified bundle, so entries are
//! derived from the bundle rather than typed by hand.
use std::{fs, path::Path};

use serde_json::{Value, json};

use super::{read_json, sha256::sha256_hex, string_field, verify_bundle};

/// What a PASS of each suite demonstrates for coverage purposes.
fn covers(suite: &str, result: &Value) -> Value {
    match suite {
        "live_affinity" | "live_recovery" | "live_suspend_resume" => {
            json!({"providers": ["cpu_affinity_profile"]})
        }
        "live_game_runnable" => json!({"objectives": ["GameRunnableLatency"]}),
        "live_kernel_compat" if result["compatibility"] != "ObservedIncompatible" => {
            json!({"kernel_capabilities": ["scheduler_runnable_latency"]})
        }
        _ => json!({}),
    }
}

/// The index entry for `bundle`, derived only after the bundle fully verifies. Release
/// acceptance consumes this directly so an unverified or tampered bundle never yields an entry.
pub fn verified_index_entry(bundle: &Path) -> anyhow::Result<Value> {
    verify_bundle(bundle)?;
    let receipt = read_json(&bundle.join("receipt.json"))?;
    let environment = read_json(&bundle.join("environment.json"))?;
    let result = read_json(&bundle.join("artifacts/result.json"))?;
    let suite = string_field(&receipt, "suite")?;
    let mut entry = json!({
        "suite": suite,
        "evidence_kind": receipt["evidence_kind"],
        "status": receipt["status"],
        "capture_id": receipt["capture_id"],
        "source_commit": receipt["source"]["commit"],
        "manifest_sha256": sha256_hex(&fs::read(bundle.join("MANIFEST.sha256"))?),
        "covers": covers(suite, &result),
    });
    if suite == "live_kernel_compat" {
        entry["kernel_release"] = environment["kernel_release"].clone();
        entry["compatibility"] = result["compatibility"].clone();
    }
    Ok(entry)
}

pub fn index_entry(bundle: &Path) -> anyhow::Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&verified_index_entry(bundle)?)?
    );
    Ok(())
}
