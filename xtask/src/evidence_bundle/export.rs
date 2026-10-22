//! Raw live-acceptance output → canonical bundle: sanitize first, scan second,
//! manifest last, then re-verify with the independent verifier before publishing.
use std::{collections::BTreeSet, fs, path::Path};

use anyhow::{Context, bail, ensure};
use serde_json::{Value, json};

use super::{
    BUNDLE_SCHEMA_VERSION, ENVIRONMENT_FIELDS, EvidenceKind, MAX_FILE_BYTES,
    REQUIRED_ENVIRONMENT_FIELDS, field,
    privacy::{PLACEHOLDERS, Redactor},
    read_json,
    sha256::sha256_hex,
    string_field, summary_json,
    verify::{forbidden_name, list_files, run_checks},
};

fn is_log(relative: &str) -> bool {
    ["stdout", "stderr", "log", "txt"]
        .iter()
        .any(|suffix| relative.ends_with(&format!(".{suffix}")))
}

fn first_line(path: &str) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|text| text.trim().to_owned())
}

fn local_redactor(root: &Path, raw: &Path, environment: &Value) -> Redactor {
    // Tokens that carry provenance must survive even if they equal a local name.
    let protected = ["distribution", "kernel_release", "cpu_model", "rust"]
        .iter()
        .filter_map(|name| environment[*name].as_str())
        .flat_map(|text| {
            text.split(|c: char| !(c.is_ascii_alphanumeric() || "_-.".contains(c)))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect::<BTreeSet<_>>();
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .ok();
    Redactor::for_machine(
        &raw.to_string_lossy(),
        &root.to_string_lossy(),
        std::env::var("HOME").ok().as_deref(),
        user.as_deref(),
        first_line("/proc/sys/kernel/hostname").as_deref(),
        &protected,
    )
}

pub fn export_bundle(root: &Path, raw: &Path, out: &Path) -> anyhow::Result<()> {
    ensure!(
        !out.exists(),
        "bundle output {} already exists",
        out.display()
    );
    let raw = fs::canonicalize(raw).with_context(|| format!("raw evidence {}", raw.display()))?;
    let result = read_json(&raw.join("result.json"))
        .context("raw evidence has no result.json; the suite did not finish")?;
    let environment = read_json(&raw.join("environment.json")).context(
        "raw evidence has no environment.json (e.g. a BLOCKED preflight); nothing to export",
    )?;
    for name in REQUIRED_ENVIRONMENT_FIELDS {
        if field(&environment, name).is_err() {
            bail!(
                "raw evidence lacks provenance field {name}; rerun the suite from the exact \
                 committed source (historical metadata is never reconstructed)"
            );
        }
    }
    let kind = EvidenceKind::parse(string_field(&environment, "evidence_kind")?)?;
    let redactor = local_redactor(root, &raw, &environment);
    let partial = out.with_extension("partial");
    if partial.exists() {
        fs::remove_dir_all(&partial)?;
    }
    fs::create_dir_all(partial.join("logs"))?;
    fs::create_dir_all(partial.join("artifacts"))?;
    let built = (|| -> anyhow::Result<()> {
        // 1. Sanitize and stage every regular text file.
        let (mut logs, mut artifacts) = (0_usize, 0_usize);
        for relative in list_files(&raw)? {
            ensure!(!forbidden_name(&relative), "refusing to export {relative}");
            if relative == "environment.json" {
                continue;
            }
            let bytes = fs::read(raw.join(&relative))?;
            ensure!(
                bytes.len() as u64 <= MAX_FILE_BYTES,
                "{relative} exceeds the bundle size limit"
            );
            let text = String::from_utf8(bytes)
                .ok()
                .filter(|text| !text.contains('\0'))
                .with_context(|| format!("refusing non-text artifact {relative}"))?;
            let clean = redactor.sanitize(&text);
            let target = if is_log(&relative) {
                logs += 1;
                partial.join("logs").join(&relative)
            } else {
                artifacts += 1;
                partial.join("artifacts").join(&relative)
            };
            fs::create_dir_all(target.parent().context("no parent")?)?;
            fs::write(target, clean)?;
        }
        // 2. Whitelisted, sanitized environment.
        let mut portable = serde_json::Map::new();
        for name in ENVIRONMENT_FIELDS {
            if let Some(value) = environment.get(name) {
                portable.insert(name.to_owned(), value.clone());
            }
        }
        let environment_text = redactor.sanitize(&serde_json::to_string_pretty(&portable)?);
        fs::write(partial.join("environment.json"), environment_text)?;
        // 3. Receipt.
        let cases = result["cases"]
            .as_array()
            .map(|cases| {
                cases
                    .iter()
                    .map(|case| {
                        json!({
                            "name": case["checkpoint"].as_str().or(case["name"].as_str()),
                            "status": case["status"],
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let receipt = json!({
            "schema_version": BUNDLE_SCHEMA_VERSION,
            "suite": environment["suite"],
            "evidence_kind": kind.as_str(),
            "status": result["status"],
            "capture_id": environment["capture_id"],
            "captured_unix_ms": environment["captured_unix_ms"],
            "finished_unix_ms": result["finished_unix_ms"],
            "source": {
                "commit": environment["commit"],
                "dirty": environment["source_dirty"],
                "matches_binary": environment["source_matches_binary"],
            },
            "binary": {"version": environment["binary_version"], "sha256": environment["binary_sha256"]},
            "cargo_lock_sha256": environment["cargo_lock_sha256"],
            "artifact_schema": environment["artifact_schema"],
            "environment": "environment.json",
            "scope": result["scope"],
            "limitations": result["scope"],
            "not_claimed": result["not_claimed"],
            "error": result["error"],
            "cases": cases,
            "files": {"logs": logs, "artifacts": artifacts},
            "sanitization": {
                "placeholders": PLACEHOLDERS.iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<serde_json::Map<_, _>>(),
                "local_identity_sha256": redactor.literal_hashes(),
            },
        });
        fs::write(
            partial.join("receipt.json"),
            redactor.sanitize(&serde_json::to_string_pretty(&receipt)?),
        )?;
        // 4. Pre-manifest validation record, README, then the manifest last.
        fs::write(partial.join("README.md"), readme(&receipt))?;
        let pre = run_checks(&partial)?
            .into_iter()
            .filter(|(name, _, _)| name != "required_files" && name != "manifest")
            .collect::<Vec<_>>();
        fs::write(
            partial.join("validation.json"),
            serde_json::to_string_pretty(&json!({
                "schema_version": BUNDLE_SCHEMA_VERSION,
                "note": "checks run by the exporter before the manifest existed; \
                         `xtask evidence-verify` recomputes everything independently",
                "checks": summary_json(&pre),
            }))?,
        )?;
        let mut manifest = String::new();
        for relative in list_files(&partial)? {
            if relative == "MANIFEST.sha256" {
                continue;
            }
            manifest.push_str(&super::manifest_line(
                &sha256_hex(&fs::read(partial.join(&relative))?),
                &relative,
            ));
        }
        fs::write(partial.join("MANIFEST.sha256"), manifest)?;
        // 5. The independent verifier must accept what we are about to publish.
        super::fail_if_any(&run_checks(&partial)?)
    })();
    if let Err(error) = built {
        let _ = fs::remove_dir_all(&partial);
        return Err(error.context("evidence export refused; nothing was published"));
    }
    fs::rename(&partial, out)?;
    println!("evidence_exported={}", out.display());
    Ok(())
}

fn readme(receipt: &Value) -> String {
    format!(
        "# Evidence bundle: {suite}\n\n\
         - status: **{status}**\n- evidence kind: `{kind}`\n- capture: `{capture}`\n\
         - source commit: `{commit}`\n- binary sha256: `{binary}`\n\n\
         Verify with `cargo run -p xtask -- evidence-verify <this directory>`.\n\
         Local paths, account and host names are replaced by documented placeholders \
         (see `receipt.json` → `sanitization`); provenance values are never redacted.\n\
         See `not_claimed` in `receipt.json` for what this evidence does not establish.\n",
        suite = receipt["suite"].as_str().unwrap_or("?"),
        status = receipt["status"].as_str().unwrap_or("?"),
        kind = receipt["evidence_kind"].as_str().unwrap_or("?"),
        capture = receipt["capture_id"].as_str().unwrap_or("?"),
        commit = receipt["source"]["commit"].as_str().unwrap_or("?"),
        binary = receipt["binary"]["sha256"].as_str().unwrap_or("?"),
    )
}
