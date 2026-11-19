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

/// The recorded integration coverage (every integration, with its own dispositions and evidence
/// files), embedded from the repository so the matrix cannot drift from the ledger it cites.
const COVERAGE: &str = include_str!("../../../../docs/integration-coverage.json");

/// Each (source, factor) cell with what can honestly be said of it: not applicable, unavailable, not
/// observable, or a control that exists but is not functionally proven. Never "proven": no factor has
/// a functional receipt.
fn feature_dispositions() -> Vec<Value> {
    super::features::feature_dispositions()
        .into_iter()
        .map(|(source, factor, disposition)| {
            json!({
                "source": format!("{source:?}"),
                "factor": format!("{factor:?}"),
                "disposition": disposition,
            })
        })
        .collect()
}

fn integration_matrix() -> Value {
    let Ok(coverage) = serde_json::from_str::<Value>(COVERAGE) else {
        return json!({ "status": "CoverageLedgerUnreadable" });
    };
    let rows: Vec<Value> = coverage
        .get("entries")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .map(|entry| {
                    json!({
                        "id": entry.get("id"),
                        "tools": entry.get("tools"),
                        "authority": entry.get("authority"),
                        "dispositions": entry.get("dispositions"),
                        "evidence": entry.get("evidence"),
                        "runtime_availability": entry.get("runtime_availability"),
                        "not_wired": entry.get("not_wired"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    json!({
        "source_sha": coverage.get("source_sha"),
        "recorded": coverage.get("recorded"),
        "count": rows.len(),
        "note": "dispositions are the ledger's own; none is a result of this campaign unless a study below qualifies it",
        "integrations": rows,
    })
}

/// A qualified action study without its receipts is not a qualified action.
fn action_status(status: &'static str, receipts: &[Value]) -> &'static str {
    if status == "Qualified" && receipts.is_empty() {
        "ReceiptsMissing"
    } else {
        status
    }
}

/// Count reconstructed child reports when a funnel has run; the flat parent contains no child
/// attempts. If any child is missing or cannot be reconstructed, the known total is only a lower
/// bound and must never be presented as the campaign's complete total.
fn completed_attempts(report: &Value, response: &Value) -> String {
    let children = response
        .pointer("/child_reports/children")
        .and_then(Value::as_array);
    let missing = response
        .pointer("/child_reports/missing")
        .and_then(Value::as_array);
    if let (Some(children), Some(missing)) = (children, missing)
        && (!children.is_empty() || !missing.is_empty())
    {
        let mut incomplete = !missing.is_empty();
        let mut total = 0u64;
        for child in children {
            let count = (child.get("reconstruction") == Some(&Value::Bool(true)))
                .then(|| {
                    child
                        .pointer("/report/attempts_completed")
                        .and_then(Value::as_u64)
                })
                .flatten();
            match count.and_then(|count| total.checked_add(count)) {
                Some(sum) => total = sum,
                None => incomplete = true,
            }
        }
        return if incomplete {
            format!("at least {total} (verified child reports; some child evidence is unavailable)")
        } else {
            format!("{total} (verified child reports)")
        };
    }
    if (children.is_none() || missing.is_none())
        && studies(response)
            .iter()
            .any(|study| matches!(result_of(study), "qualified" | "unqualified" | "failed"))
    {
        return "unknown (child evidence is unavailable)".to_owned();
    }
    report
        .get("attempts_completed")
        .and_then(Value::as_u64)
        .map_or_else(|| "unknown".to_owned(), |count| count.to_string())
}

fn summary(report: &Value, response: &Value) -> String {
    let verdict = response.get("verdict").cloned().unwrap_or(Value::Null);
    let flag = |name: &str| verdict.get(name).and_then(Value::as_bool).unwrap_or(false);
    let all = studies(response);
    let decided = all
        .iter()
        .filter(|study| result_of(study) != "undecided")
        .count();
    let gates: String = verdict
        .get("gates")
        .and_then(Value::as_array)
        .map(|gates| {
            gates
                .iter()
                .map(|gate| {
                    let blockers = gate["blockers"]
                        .as_array()
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(Value::as_str)
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_default();
                    format!(
                        "- {}: {}{}\n",
                        gate["gate"].as_str().unwrap_or("?"),
                        if gate["accepted"] == true {
                            "accepted"
                        } else {
                            "not accepted"
                        },
                        if blockers.is_empty() {
                            String::new()
                        } else {
                            format!(" ({blockers})")
                        },
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    format!(
        "# Campaign summary\n\n\
         Disposition: {}\n\n\
         - reconstruction passed: {}\n- raw native files verified: {}\n\
         - loaded runtime qualified: {}\n- child studies qualified: {}\n\
         - child reports verified: {}\n- restoration verified: {}\n\
         - final gate qualified: {}\n\n\
         Child studies decided: {decided} of {}.\n\
         Attempts completed: {}.\n\n\
         Gates:\n{gates}\n\
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
        completed_attempts(report, response),
    )
}

/// The verified children, joined to their planned study (profile and stage by digest prefix).
fn child_rows(response: &Value, with_report: bool) -> Vec<Value> {
    let planned = studies(response);
    response
        .pointer("/child_reports/children")
        .and_then(Value::as_array)
        .map(|children| {
            children
                .iter()
                .map(|child| {
                    let name = child.get("study").and_then(Value::as_str).unwrap_or("");
                    let study = planned.iter().find(|study| {
                        study
                            .get("study_sha256")
                            .and_then(Value::as_str)
                            .is_some_and(|digest| digest.starts_with(name))
                    });
                    let mut row = json!({
                        "study": name,
                        "profile": study.and_then(|study| study.get("profile")),
                        "stage": study.map(|study| stage_of(study)),
                        "reconstruction": child.get("reconstruction"),
                        "raw_files_verified": child.get("raw_files_verified"),
                        "problem": child.get("problem"),
                        "action_receipts": child.get("action_receipts"),
                        "prefix_cohort": child.get("prefix_cohort"),
                        "replication": child.get("replication"),
                        // What the resident runtime saw per attempt and the process each completed
                        // attempt's sealed native result names, so the join can be re-checked.
                        "telemetry_receipts": child.get("telemetry_receipts"),
                        "completed_attempts": child.get("completed_attempts"),
                        "completed_processes": child.get("completed_processes"),
                    });
                    if with_report {
                        row["report"] = child.get("report").cloned().unwrap_or(Value::Null);
                    }
                    row
                })
                .collect()
        })
        .unwrap_or_default()
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
    // Each applied action's receipt: the frozen process, the kernel state before the apply and the
    // state after the restoration was accepted, recorded by the action child beside its study.
    // A safely applied and restored action is one claim; that it improved the protected objective is
    // another, which needs the study's own adequately sampled off/on comparison (`pairs`, verdict).
    let comparison: Vec<Value> = child_rows(response, true)
        .iter()
        .filter(|row| row.get("profile").and_then(Value::as_str) == Some("long"))
        .map(|row| {
            let result = row
                .pointer("/report/arms/0")
                .cloned()
                .unwrap_or(Value::Null);
            json!({
                "study": row.get("study"),
                "status": result.get("status"),
                "pairs": result.get("pairs"),
                "numeric_verdict": result.get("numeric_verdict"),
            })
        })
        .collect();
    let benefit_demonstrated = !comparison.is_empty()
        && comparison
            .iter()
            .all(|item| item["numeric_verdict"] == "improved");
    let receipts: Vec<Value> = child_rows(response, false)
        .iter()
        .filter(|row| row.get("profile").and_then(Value::as_str) == Some("long"))
        .filter_map(|row| row.get("action_receipts").cloned())
        .filter(|receipts| receipts.as_array().is_some_and(|items| !items.is_empty()))
        .collect();
    let mut files: Vec<(&'static str, Vec<u8>)> = vec![
        (
            "campaign-summary.md",
            summary(report, response).into_bytes(),
        ),
        ("report.json", pretty(report)),
        (
            "capability-matrix.json",
            pretty(&json!({
                "integration_coverage": integration_matrix(),
                "feature_dispositions": feature_dispositions(),
                "child_studies": response.get("child_studies"),
            })),
        ),
        (
            "proton-comparison.json",
            pretty(&json!({
                "note": "in funnel mode the comparison is in the verified child reports; the flat campaign report covers only the flat schedule",
                "flat_campaign_report": report,
                "child_reports": child_rows(response, true),
            })),
        ),
        (
            "diagnosis-oracle.json",
            pretty(&json!({
                "status": status_of(&long),
                "studies": long,
                // The oracle's own scoring of frozen ground truth against the controller's captured
                // diagnosis, or why none exists; it is not inferred from the long study's status.
                "scored": response.get("diagnosis_oracle").cloned().unwrap_or(Value::Null),
                "note": "the diagnosis is judged only by scored cases whose ground truth was frozen \
                         before the output was captured (see `scored`) and by a qualified live \
                         CP-LONG study",
            })),
        ),
        (
            "action-transactions.json",
            pretty(&json!({
                "status": action_status(status_of(&long), &receipts),
                "studies": long,
                "receipts": receipts,
                "outcome_comparison": comparison,
                "benefit_demonstrated": benefit_demonstrated,
                "note": "status says whether the action was safely applied and restored; benefit_demonstrated says only whether the off/on comparison found an improvement",
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
                "child_reports": child_rows(response, false),
                "children_missing": response.pointer("/child_reports/missing"),
            })),
        ),
        (
            "restore-proof.json",
            pretty(&json!({
                "restoration_verified": verdict.get("restoration_verified"),
                "basis": "true only when every planned action study qualified with a verified child whose \
                          kernel-state receipts are each attributable to a completed attempt treated On, \
                          every treated attempt carries one, and no research prefix still carries an \
                          unrestored settings edit; unknown is false and it is never inferred from other \
                          statuses. The receipts are in action-transactions.json.",
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

/// Reopen every published artifact and compare it with the deterministic bundle and manifest.
pub(crate) fn verify_written(
    dir: &File,
    uid: u32,
    report: &Value,
    response: &Value,
    name: &str,
) -> anyhow::Result<()> {
    use std::os::{
        fd::{AsFd, AsRawFd},
        unix::fs::MetadataExt,
    };
    let files = build(report, response);
    let manifest = files
        .last()
        .ok_or_else(|| anyhow::anyhow!("bundle manifest missing"))?;
    let expected_name = format!(
        "bundle-{}",
        &stutter_core::hash::sha256_hex(&manifest.1)[..16]
    );
    anyhow::ensure!(name == expected_name, "bundle identity differs");
    let bundle = File::from(crate::syscall::openat_dir_nofollow(dir.as_fd(), name)?);
    let metadata = bundle.metadata()?;
    anyhow::ensure!(
        metadata.uid() == uid && metadata.mode() & 0o077 == 0,
        "untrusted bundle directory"
    );
    let actual = std::fs::read_dir(format!("/proc/self/fd/{}", bundle.as_raw_fd()))?
        .collect::<std::io::Result<Vec<_>>>()?
        .len();
    anyhow::ensure!(actual == files.len(), "bundle entries differ");
    for (file, bytes) in files {
        anyhow::ensure!(
            super::report_store::read_private(&bundle, file, uid)? == bytes,
            "bundle artifact differs"
        );
    }
    Ok(())
}

/// Verify a historical bundle from its manifest, including its report and recorded verdict.
pub(crate) fn verify_manifest(
    dir: &File,
    uid: u32,
    name: &str,
    report: &Value,
    verdict: &Value,
) -> anyhow::Result<()> {
    use std::{
        collections::BTreeSet,
        os::{
            fd::{AsFd, AsRawFd},
            unix::fs::MetadataExt,
        },
    };
    anyhow::ensure!(
        name.len() == 23
            && name.starts_with("bundle-")
            && name[7..]
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "invalid bundle name"
    );
    let bundle = File::from(crate::syscall::openat_dir_nofollow(dir.as_fd(), name)?);
    let metadata = bundle.metadata()?;
    anyhow::ensure!(
        metadata.uid() == uid && metadata.mode() & 0o077 == 0,
        "untrusted bundle directory"
    );
    let manifest = super::report_store::read_private(&bundle, "manifest-and-hashes.txt", uid)?;
    anyhow::ensure!(
        name == format!(
            "bundle-{}",
            &stutter_core::hash::sha256_hex(&manifest)[..16]
        ),
        "bundle identity differs"
    );
    let expected: BTreeSet<_> = build(&Value::Null, &Value::Null)
        .into_iter()
        .map(|(file, _)| file)
        .filter(|file| *file != "manifest-and-hashes.txt")
        .collect();
    let mut seen = BTreeSet::new();
    for line in std::str::from_utf8(&manifest)?.lines() {
        let (digest, file) = line
            .split_once("  ")
            .ok_or_else(|| anyhow::anyhow!("invalid bundle manifest"))?;
        anyhow::ensure!(
            expected.contains(file) && seen.insert(file.to_owned()),
            "unexpected bundle entry"
        );
        let bytes = super::report_store::read_private(&bundle, file, uid)?;
        anyhow::ensure!(
            stutter_core::hash::sha256_hex(&bytes) == digest,
            "bundle artifact hash differs"
        );
        if file == "report.json" {
            anyhow::ensure!(
                serde_json::from_slice::<Value>(&bytes)? == *report,
                "bundle report differs"
            );
        }
        if file == "verify-result.json" {
            anyhow::ensure!(
                serde_json::from_slice::<Value>(&bytes)? == *verdict,
                "bundle verdict differs"
            );
        }
    }
    let actual = std::fs::read_dir(format!("/proc/self/fd/{}", bundle.as_raw_fd()))?
        .collect::<std::io::Result<Vec<_>>>()?
        .len();
    anyhow::ensure!(
        seen.len() == expected.len() && actual == expected.len() + 1,
        "bundle entries differ"
    );
    Ok(())
}

/// Write the bundle for a verified report and say what happened. A verdict with no durable bundle is
/// not a final one, so a failure is returned for the response to carry, not merely logged.
pub fn write_reporting(
    dir: &File,
    report: &impl serde::Serialize,
    response: &Value,
) -> Result<String, String> {
    let report = serde_json::to_value(report).map_err(|error| format!("{error:#}"))?;
    write(dir, &report, response).map_err(|error| {
        log::warn!("final bundle not written: {error:#}");
        format!("{error:#}")
    })
}

#[cfg(test)]
#[path = "final_bundle_tests.rs"]
mod tests;
