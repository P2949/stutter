//! The final artifact bundle PLAN section 1 names, derived only from what a verification already
//! established. Nothing here measures or concludes anything new: a file whose evidence does not
//! exist says so (`NotEvaluated`, a blocked reason, `restoration_verified: false`) rather than being
//! omitted or filled in, and the manifest binds every other file by SHA-256.

use std::fs::File;

use serde_json::{Value, json};

use super::report_store::write_new;

fn pretty(value: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(value).unwrap_or_default();
    bytes.push(b'\n');
    bytes
}

fn stage_of(study: &Value) -> &str {
    study
        .get("stage")
        .and_then(|stage| stage.get("stage").or(Some(stage)))
        .and_then(Value::as_str)
        .unwrap_or("")
}

fn studies(response: &Value) -> Vec<&Value> {
    response
        .get("child_studies")
        .and_then(Value::as_array)
        .map(|items| items.iter().collect())
        .unwrap_or_default()
}

fn result_of(study: &Value) -> &str {
    study
        .get("result")
        .and_then(Value::as_str)
        .unwrap_or("undecided")
}

fn long_action<'a>(all: &[&'a Value]) -> Vec<&'a Value> {
    all.iter()
        .copied()
        .filter(|study| study.get("profile").and_then(Value::as_str) == Some("long"))
        .collect()
}

fn status_of(group: &[&Value]) -> &'static str {
    if group.is_empty() {
        "NotPlanned"
    } else if group.iter().all(|study| result_of(study) == "qualified") {
        "Qualified"
    } else if group.iter().any(|study| result_of(study) == "undecided") {
        "NotEvaluated"
    } else {
        "NotQualified"
    }
}

fn summary(report: &Value, response: &Value) -> String {
    let verdict = response.get("verdict").cloned().unwrap_or(Value::Null);
    let flag = |name: &str| verdict.get(name).and_then(Value::as_bool).unwrap_or(false);
    let all = studies(response);
    let decided = all
        .iter()
        .filter(|study| result_of(study) != "undecided")
        .count();
    format!(
        "# Campaign summary\n\n\
         Disposition: {}\n\n\
         - reconstruction passed: {}\n- raw native files verified: {}\n\
         - loaded runtime qualified: {}\n- child studies qualified: {}\n\
         - child reports verified: {}\n- restoration verified: {}\n\
         - final gate qualified: {}\n\n\
         Child studies decided: {decided} of {}.\n\
         Attempts completed: {}.\n\n\
         This summary restates verified statuses only; a false status means that evidence was not \
         established, not that it failed.\n",
        verdict
            .get("disposition")
            .and_then(Value::as_str)
            .unwrap_or("Unverified"),
        flag("reconstruction_passed"),
        flag("raw_files_verified"),
        flag("loaded_runtime_qualified"),
        flag("child_studies_qualified"),
        flag("child_reports_verified"),
        flag("restoration_verified"),
        flag("final_gate_qualified"),
        all.len(),
        report
            .get("attempts_completed")
            .map_or_else(|| "unknown".to_owned(), ToString::to_string),
    )
}

/// Every bundle file (name, bytes) for one verified campaign, the manifest last.
pub fn build(report: &Value, response: &Value) -> Vec<(&'static str, Vec<u8>)> {
    let verdict = response.get("verdict").cloned().unwrap_or(Value::Null);
    let all = studies(response);
    let long = long_action(&all);
    let blocked: Vec<_> = all
        .iter()
        .filter(|study| result_of(study) != "qualified")
        .map(|study| {
            json!({
                "study_sha256": study.get("study_sha256"),
                "profile": study.get("profile"),
                "stage": stage_of(study),
                "result": result_of(study),
                "reason": study.get("reason"),
            })
        })
        .collect();
    let mut files: Vec<(&'static str, Vec<u8>)> = vec![
        (
            "campaign-summary.md",
            summary(report, response).into_bytes(),
        ),
        ("report.json", pretty(report)),
        (
            "capability-matrix.json",
            pretty(&json!({ "child_studies": response.get("child_studies") })),
        ),
        ("proton-comparison.json", pretty(report)),
        (
            "diagnosis-oracle.json",
            pretty(&json!({
                "status": status_of(&long),
                "studies": long,
                "note": "the diagnosis is judged only by a qualified live CP-LONG study",
            })),
        ),
        (
            "action-transactions.json",
            pretty(&json!({
                "status": status_of(&long),
                "studies": long,
                "note": "transactions come from the normal controller and root worker only",
            })),
        ),
        (
            "excluded-and-blocked.json",
            pretty(&json!({ "studies": blocked })),
        ),
        (
            "source-integrity.json",
            pretty(&json!({
                "intent_sha256": response.get("intent_sha256"),
                "raw_native_files": response.get("raw_native_files"),
                "raw_files_verified": verdict.get("raw_files_verified"),
                "loaded_runtime_qualified": verdict.get("loaded_runtime_qualified"),
                "child_reports": response.get("child_reports"),
            })),
        ),
        (
            "restore-proof.json",
            pretty(&json!({
                "restoration_verified": verdict.get("restoration_verified"),
                "basis": "no physical restoration verifier exists; never inferred from other statuses",
            })),
        ),
        ("verify-result.json", pretty(&verdict)),
    ];
    let manifest: String = files
        .iter()
        .map(|(name, bytes)| format!("{}  {name}\n", stutter_core::hash::sha256_hex(bytes)))
        .collect();
    files.push(("manifest-and-hashes.txt", manifest.into_bytes()));
    files
}

/// Write the bundle under `dir` as `bundle-<first 16 hex of its manifest digest>/`: the identity of a
/// bundle is its content, so a repeated identical verification finds every file already there and
/// compares it byte for byte (a mismatch is refused, never accepted as stale output), and a changed
/// verification result is a new bundle beside the old one, which is never overwritten.
pub fn write(dir: &File, report: &Value, response: &Value) -> anyhow::Result<String> {
    use std::os::fd::AsFd;
    let files = build(report, response);
    let manifest = files
        .last()
        .map(|(_, bytes)| bytes.as_slice())
        .unwrap_or(&[]);
    let name = format!("bundle-{}", &stutter_core::hash::sha256_hex(manifest)[..16]);
    match crate::syscall::mkdir_private_at(dir.as_fd(), &name) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let bundle = File::from(crate::syscall::openat_dir_nofollow(dir.as_fd(), &name)?);
    let uid = crate::syscall::geteuid();
    for (file, bytes) in &files {
        match write_new(&bundle, file, bytes) {
            Ok(()) => {}
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|io| io.kind() == std::io::ErrorKind::AlreadyExists) =>
            {
                anyhow::ensure!(
                    super::report_store::read_private(&bundle, file, uid)? == *bytes,
                    "{name}/{file} differs from the verification it claims to record"
                );
            }
            Err(error) => return Err(error),
        }
    }
    Ok(name)
}

/// Write the bundle for a verified report; a failure is logged and is not a verdict.
pub fn write_logged(dir: &File, report: &impl serde::Serialize, response: &Value) {
    let report = serde_json::to_value(report).unwrap_or_default();
    if let Err(error) = write(dir, &report, response) {
        log::warn!("final bundle not written: {error:#}");
    }
}

#[cfg(test)]
#[path = "final_bundle_tests.rs"]
mod tests;
