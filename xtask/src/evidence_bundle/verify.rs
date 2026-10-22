//! Independent bundle verifier: trusts nothing the exporter claimed.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use anyhow::{Context, ensure};
use serde_json::Value;

use super::{
    BUNDLE_SCHEMA_VERSION, ENVIRONMENT_FIELDS, EvidenceKind, KNOWN_SUITES, MAX_FILE_BYTES,
    REQUIRED_ENVIRONMENT_FIELDS, STATUSES, TOP_LEVEL_DIRS, TOP_LEVEL_FILES, ensure_relative,
    fail_if_any, field, is_hex, privacy, read_json, sha256::sha256_hex, string_field,
};

/// Names that must never appear in publishable evidence, whatever their content.
const FORBIDDEN_NAMES: [&str; 8] = [
    ".env",
    "id_rsa",
    "id_ed25519",
    "known_hosts",
    "authorized_keys",
    ".netrc",
    "credentials",
    ".git-credentials",
];
const FORBIDDEN_SUFFIXES: [&str; 12] = [
    ".pem", ".key", ".p12", ".pfx", ".core", ".pcap", ".data", ".zst", ".gz", ".tar", ".bin", ".o",
];

/// Every regular file under `root` (relative, sorted). Symlinks are an error.
pub(super) fn list_files(root: &Path) -> anyhow::Result<Vec<String>> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) -> anyhow::Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            let relative = entry
                .path()
                .strip_prefix(base)?
                .to_string_lossy()
                .into_owned();
            ensure!(!kind.is_symlink(), "symlink in bundle: {relative}");
            if kind.is_dir() {
                walk(base, &entry.path(), out)?;
            } else {
                ensure!(kind.is_file(), "special file in bundle: {relative}");
                out.push(relative);
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(root, root, &mut files)?;
    files.sort();
    Ok(files)
}

pub(super) fn forbidden_name(relative: &str) -> bool {
    let name = relative.rsplit('/').next().unwrap_or(relative);
    FORBIDDEN_NAMES.contains(&name)
        || FORBIDDEN_SUFFIXES
            .iter()
            .any(|suffix| name.ends_with(suffix))
}

fn string_set(value: &Value, name: &str) -> anyhow::Result<BTreeSet<String>> {
    Ok(field(value, name)?
        .as_array()
        .with_context(|| format!("{name} is not an array"))?
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect())
}

fn check_receipt(receipt: &Value) -> anyhow::Result<String> {
    ensure!(
        receipt["schema_version"].as_u64() == Some(BUNDLE_SCHEMA_VERSION),
        "unsupported bundle schema_version"
    );
    let suite = string_field(receipt, "suite")?;
    ensure!(KNOWN_SUITES.contains(&suite), "unknown suite {suite}");
    EvidenceKind::parse(string_field(receipt, "evidence_kind")?)?;
    ensure!(
        STATUSES.contains(&string_field(receipt, "status")?),
        "status is not one of {STATUSES:?}"
    );
    string_field(receipt, "capture_id")?;
    ensure!(
        field(receipt, "captured_unix_ms")?.is_u64()
            && field(receipt, "finished_unix_ms")?.is_u64(),
        "capture start/end must be unix milliseconds"
    );
    ensure!(
        receipt["finished_unix_ms"].as_u64() >= receipt["captured_unix_ms"].as_u64(),
        "capture ends before it starts"
    );
    let source = field(receipt, "source")?;
    ensure!(is_hex(string_field(source, "commit")?, 40), "source.commit");
    ensure!(
        field(source, "dirty")?.is_boolean() && field(source, "matches_binary")?.is_boolean(),
        "source.dirty/matches_binary must be booleans"
    );
    let binary = field(receipt, "binary")?;
    string_field(binary, "version")?;
    ensure!(is_hex(string_field(binary, "sha256")?, 64), "binary.sha256");
    ensure!(
        is_hex(string_field(receipt, "cargo_lock_sha256")?, 64),
        "cargo_lock_sha256"
    );
    ensure!(
        field(receipt, "artifact_schema")?.is_object(),
        "artifact_schema must be an object"
    );
    ensure!(
        !string_set(receipt, "not_claimed")?.is_empty(),
        "explicit limitations (not_claimed) are required"
    );
    ensure!(
        field(receipt, "cases").is_ok_and(Value::is_array),
        "cases must be an array"
    );
    let sanitization = field(receipt, "sanitization")?;
    ensure!(
        field(sanitization, "placeholders")?.is_object()
            && string_set(sanitization, "local_identity_sha256").is_ok(),
        "sanitization record incomplete"
    );
    Ok(format!("suite={suite}"))
}

fn check_status(receipt: &Value, environment: &Value) -> anyhow::Result<String> {
    let status = string_field(receipt, "status")?;
    let error = receipt.get("error").filter(|error| !error.is_null());
    let cases = field(receipt, "cases")?.as_array().context("cases")?;
    let all_cases_pass = cases.iter().all(|case| case["status"] == "PASS");
    match status {
        "PASS" => {
            ensure!(error.is_none(), "PASS receipt carries an error");
            ensure!(all_cases_pass, "PASS receipt contains a non-PASS case");
            ensure!(
                receipt["source"]["dirty"] == false
                    && receipt["source"]["matches_binary"] == true
                    && environment["source_dirty"] == false,
                "PASS requires a clean source whose binary matches"
            );
        }
        _ => ensure!(
            error.is_some(),
            "{status} receipt must explain itself in error"
        ),
    }
    Ok(format!("status={status} cases={}", cases.len()))
}

fn check_agreement(receipt: &Value, environment: &Value, result: &Value) -> anyhow::Result<String> {
    for key in ["suite", "evidence_kind", "capture_id"] {
        ensure!(
            receipt[key] == environment[key] && environment[key] == result[key],
            "{key} differs across receipt/environment/result: {} / {} / {}",
            receipt[key],
            environment[key],
            result[key]
        );
    }
    ensure!(
        receipt["source"]["commit"] == environment["commit"]
            && receipt["binary"]["sha256"] == environment["binary_sha256"]
            && receipt["binary"]["version"] == environment["binary_version"]
            && receipt["cargo_lock_sha256"] == environment["cargo_lock_sha256"]
            && receipt["status"] == result["status"],
        "provenance or status differs between receipt, environment and result"
    );
    Ok("suite/evidence_kind/capture/provenance agree".to_owned())
}

fn check_environment(environment: &Value) -> anyhow::Result<String> {
    let object = environment
        .as_object()
        .context("environment is not an object")?;
    for name in REQUIRED_ENVIRONMENT_FIELDS {
        field(environment, name)?;
    }
    let unexpected = object
        .keys()
        .filter(|key| !ENVIRONMENT_FIELDS.contains(&key.as_str()))
        .collect::<Vec<_>>();
    ensure!(
        unexpected.is_empty(),
        "unexpected environment fields {unexpected:?}"
    );
    ensure!(
        is_hex(string_field(environment, "commit")?, 40),
        "environment.commit"
    );
    ensure!(
        is_hex(string_field(environment, "binary_sha256")?, 64),
        "environment.binary_sha256"
    );
    let version = string_field(environment, "binary_version")?;
    ensure!(
        version.contains(&string_field(environment, "commit")?[..7]),
        "binary_version {version:?} does not name the source commit"
    );
    Ok("required provenance present".to_owned())
}

fn collect_strings(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) => out.push(text.clone()),
        Value::Array(items) => items.iter().for_each(|item| collect_strings(item, out)),
        Value::Object(map) => map.values().for_each(|item| collect_strings(item, out)),
        _ => {}
    }
}

/// Verifies `bundle`, returning every check. Errors only on I/O of the root itself.
pub(super) fn run_checks(bundle: &Path) -> anyhow::Result<Vec<(String, bool, String)>> {
    ensure!(bundle.is_dir(), "{} is not a directory", bundle.display());
    let mut checks: Vec<(String, bool, String)> = Vec::new();
    let mut record = |name: &str, result: anyhow::Result<String>| match result {
        Ok(detail) => checks.push((name.to_owned(), true, detail)),
        Err(error) => checks.push((name.to_owned(), false, format!("{error:#}"))),
    };

    let files = list_files(bundle);
    let files_ok = files.is_ok();
    let files = files.unwrap_or_default();
    record(
        "required_files",
        (|| {
            for name in TOP_LEVEL_FILES {
                ensure!(files.iter().any(|file| file == name), "missing {name}");
            }
            Ok(format!("{} files", files.len()))
        })(),
    );
    record(
        "no_unexpected_entries",
        (|| {
            ensure!(files_ok, "bundle contains a symlink or special file");
            for file in &files {
                let top = file.split('/').next().unwrap_or(file);
                ensure!(
                    (!file.contains('/') && TOP_LEVEL_FILES.contains(&top))
                        || (file.contains('/') && TOP_LEVEL_DIRS.contains(&top)),
                    "unexpected entry {file}"
                );
                ensure!(!forbidden_name(file), "forbidden artifact {file}");
                ensure_relative(file)?;
                let size = fs::metadata(bundle.join(file))?.len();
                ensure!(size <= MAX_FILE_BYTES, "oversized artifact {file}");
                let bytes = fs::read(bundle.join(file))?;
                ensure!(
                    !bytes.contains(&0) && std::str::from_utf8(&bytes).is_ok(),
                    "non-text (raw) artifact {file}"
                );
            }
            Ok("only receipt files, logs/ and artifacts/ text".to_owned())
        })(),
    );
    record(
        "manifest",
        (|| {
            let manifest = fs::read_to_string(bundle.join("MANIFEST.sha256"))?;
            let mut listed = BTreeMap::new();
            let mut previous = String::new();
            for line in manifest.lines() {
                let (hash, path) = line.split_once("  ").context("malformed manifest line")?;
                ensure!(is_hex(hash, 64), "bad digest for {path}");
                ensure_relative(path)?;
                ensure!(
                    path > previous.as_str(),
                    "manifest not strictly sorted at {path}"
                );
                previous = path.to_owned();
                listed.insert(path.to_owned(), hash.to_owned());
            }
            let actual = files
                .iter()
                .filter(|file| *file != "MANIFEST.sha256")
                .cloned()
                .collect::<BTreeSet<_>>();
            ensure!(
                listed.keys().cloned().collect::<BTreeSet<_>>() == actual,
                "manifest and bundle contents differ"
            );
            for (path, hash) in &listed {
                ensure!(
                    sha256_hex(&fs::read(bundle.join(path))?) == *hash,
                    "digest mismatch for {path}"
                );
            }
            Ok(format!("{} digests", listed.len()))
        })(),
    );
    record(
        "json_valid",
        (|| {
            let mut count = 0;
            for file in &files {
                let text = fs::read_to_string(bundle.join(file)).unwrap_or_default();
                if file.ends_with(".json") {
                    serde_json::from_str::<Value>(&text)
                        .with_context(|| format!("invalid JSON in {file}"))?;
                    count += 1;
                } else if file.ends_with(".jsonl") {
                    for (index, line) in text.lines().filter(|l| !l.trim().is_empty()).enumerate() {
                        serde_json::from_str::<Value>(line)
                            .with_context(|| format!("invalid JSON in {file}:{}", index + 1))?;
                    }
                    count += 1;
                }
            }
            Ok(format!("{count} JSON documents"))
        })(),
    );

    let receipt = read_json(&bundle.join("receipt.json"));
    let environment = read_json(&bundle.join("environment.json"));
    let result = read_json(&bundle.join("artifacts/result.json"));
    if let Ok(receipt) = &receipt {
        record("receipt_schema", check_receipt(receipt));
    } else {
        record(
            "receipt_schema",
            Err(anyhow::anyhow!("receipt.json unreadable")),
        );
    }
    if let Ok(environment) = &environment {
        record("environment_schema", check_environment(environment));
    } else {
        record(
            "environment_schema",
            Err(anyhow::anyhow!("environment.json unreadable")),
        );
    }
    match (&receipt, &environment, &result) {
        (Ok(receipt), Ok(environment), Ok(result)) => {
            record(
                "suite_and_provenance_agreement",
                check_agreement(receipt, environment, result),
            );
            record("status_consistency", check_status(receipt, environment));
            record(
                "relative_references",
                (|| {
                    let mut strings = Vec::new();
                    collect_strings(receipt, &mut strings);
                    let reference = string_field(receipt, "environment")?;
                    ensure_relative(reference)?;
                    ensure!(
                        files.iter().any(|file| file == reference),
                        "dangling {reference}"
                    );
                    for text in strings.iter().filter(|text| text.starts_with('/')) {
                        ensure!(
                            ["/proc/", "/sys/", "/dev/"]
                                .iter()
                                .any(|ok| text.starts_with(ok)),
                            "absolute path in receipt: {text}"
                        );
                    }
                    Ok("receipt references stay inside the bundle".to_owned())
                })(),
            );
        }
        _ => {
            record(
                "suite_and_provenance_agreement",
                Err(anyhow::anyhow!(
                    "receipt, environment or artifacts/result.json unreadable"
                )),
            );
        }
    }
    record(
        "privacy_scan",
        (|| {
            let hashes = match &receipt {
                Ok(receipt) => string_set(&receipt["sanitization"], "local_identity_sha256")?,
                Err(_) => BTreeSet::new(),
            };
            let mut scanned = 0;
            for file in &files {
                let text = fs::read_to_string(bundle.join(file)).unwrap_or_default();
                let findings = privacy::scan(&text, &hashes);
                ensure!(
                    findings.is_empty(),
                    "{file}: {}",
                    findings
                        .iter()
                        .map(|finding| format!("{} at line {}", finding.kind, finding.line))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                scanned += 1;
            }
            Ok(format!("{scanned} files scanned, no findings"))
        })(),
    );
    Ok(checks)
}

pub fn verify_bundle(bundle: &Path) -> anyhow::Result<()> {
    let checks = run_checks(bundle)?;
    for (name, passed, detail) in &checks {
        println!("{} {name}: {detail}", if *passed { "PASS" } else { "FAIL" });
    }
    fail_if_any(&checks)?;
    println!("evidence_verified={}", bundle.display());
    Ok(())
}
