//! Release-candidate tasks: freeze the candidate, accept it offline and live, package, verify,
//! smoke-install and tag. Every command goes through a [`Shell`] so the whole sequence is tested
//! against a scripted machine; nothing here can move or overwrite an existing tag.
pub mod freeze;
pub mod offline;
pub mod package;
pub mod tag;

use std::path::Path;

use serde_json::{Value, json};

use super::{
    executor::RunnerOutcome,
    model::{BlockReason, FailureClass},
    repo_pr::{Checks, WaitPolicy, classify_checks},
    shell::Shell,
};

pub fn failed(class: FailureClass, stage: &str, summary: impl Into<String>) -> RunnerOutcome {
    RunnerOutcome::Failed {
        class,
        stage: stage.to_owned(),
        summary: summary.into(),
        artifacts: None,
    }
}

pub fn blocked(class: FailureClass, detail: impl Into<String>) -> RunnerOutcome {
    RunnerOutcome::Blocked(BlockReason {
        class,
        detail: detail.into(),
    })
}

/// Check runs of `commit` normalized for [`classify_checks`] (`gh` reports lowercase values).
pub fn normalized_check_runs(api_output: &str) -> Value {
    let parsed: Value = serde_json::from_str(api_output).unwrap_or(Value::Null);
    let runs = parsed["check_runs"].as_array().cloned().unwrap_or_default();
    Value::Array(
        runs.iter()
            .map(|run| {
                let upper = |key: &str| run[key].as_str().unwrap_or("").to_ascii_uppercase();
                json!({"name": run["name"], "status": upper("status"), "conclusion": upper("conclusion")})
            })
            .collect(),
    )
}

/// Waits for the exact commit's CI. Pending, skipped, cancelled, neutral and failed required
/// checks never pass. `Err` is the task outcome to record.
pub fn ci_must_be_green(
    shell: &dyn Shell,
    repo: &Path,
    commit: &str,
    wait: &WaitPolicy,
) -> Result<(), RunnerOutcome> {
    let endpoint = format!("repos/{{owner}}/{{repo}}/commits/{commit}/check-runs?per_page=100");
    for _ in 0..wait.max_polls {
        let out = shell.run("gh", &["api", &endpoint], repo);
        if !out.ok() {
            return Err(blocked(
                FailureClass::BlockedResource,
                format!("cannot read CI for {commit}: {}", out.stderr.trim()),
            ));
        }
        match classify_checks(&normalized_check_runs(&out.stdout)) {
            Checks::Green => return Ok(()),
            Checks::Red(names) => {
                return Err(failed(
                    FailureClass::SoftwareDefect,
                    "ci",
                    format!("exact-RC CI is not green: {}", names.join(", ")),
                ));
            }
            Checks::Pending => shell.sleep(wait.poll),
        }
    }
    Err(failed(
        FailureClass::Retryable,
        "ci",
        format!("CI for {commit} did not finish in time"),
    ))
}

/// The campaign's `release/` directory, created on demand.
pub fn release_dir(campaign: &Path) -> std::io::Result<std::path::PathBuf> {
    let dir = campaign.join("release");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}
