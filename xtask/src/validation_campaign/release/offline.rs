//! `rc-offline`: the existing fresh-checkout acceptance script against the exact candidate, plus
//! the candidate's own GitHub CI. The receipt is retained outside the repository.
use std::fs;

use serde_json::Value;

use super::{blocked, ci_must_be_green, failed, release_dir};
use crate::validation_campaign::{
    executor::{RunContext, RunnerOutcome, TaskRunner},
    graph::TaskSpec,
    model::FailureClass,
    repo_pr::WaitPolicy,
    shell::Shell,
};

pub struct RcOfflineRunner {
    pub shell: Box<dyn Shell>,
    pub wait: WaitPolicy,
}

impl TaskRunner for RcOfflineRunner {
    fn run(&self, context: &RunContext<'_>, _spec: &TaskSpec, _attempt: u32) -> RunnerOutcome {
        let Some(commit) = context.state.release_candidate.clone() else {
            return blocked(
                FailureClass::BlockedResource,
                "no release candidate has been frozen",
            );
        };
        let Ok(dir) = release_dir(&context.dir.0) else {
            return failed(
                FailureClass::Retryable,
                "storage",
                "cannot create the release directory",
            );
        };
        let receipt = dir.join("offline-acceptance.json");
        let _ = fs::remove_file(&receipt);
        let run = self.shell.run(
            "scripts/release/offline-acceptance.sh",
            &[&commit, &receipt.to_string_lossy()],
            context.repository,
        );
        let parsed: Option<Value> = fs::read(&receipt)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());
        let Some(parsed) = parsed else {
            return failed(
                FailureClass::Retryable,
                "acceptance",
                format!(
                    "the acceptance script wrote no receipt: {}",
                    run.stderr.trim()
                ),
            );
        };
        if !run.ok()
            || parsed["status"] != "PASS"
            || parsed["commit"] != commit.as_str()
            || parsed["fresh_checkout"] != true
        {
            // A gate can flake under load: rerun the same candidate within the retry budget;
            // a failure that survives it is a defect.
            let failing = parsed["steps"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|step| step["status"] != "PASS")
                .filter_map(|step| step["name"].as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return failed(
                FailureClass::Retryable,
                "acceptance",
                format!(
                    "offline acceptance of {commit} did not pass (status {}; failing: {failing}): {}",
                    parsed["status"],
                    run.stderr
                        .trim()
                        .lines()
                        .rev()
                        .take(12)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect::<Vec<_>>()
                        .join(" | ")
                ),
            );
        }
        if let Err(outcome) =
            ci_must_be_green(self.shell.as_ref(), context.repository, &commit, &self.wait)
        {
            return outcome;
        }
        RunnerOutcome::Passed {
            artifacts: Some(context.dir.relative(&dir)),
            receipt: Some(context.dir.relative(&receipt)),
            limitations: Vec::new(),
            fingerprint: Box::default(),
            updates: Vec::new(),
        }
    }
}
