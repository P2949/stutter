//! `tag`: the last task. It re-checks everything the policy demands (strict campaign verification,
//! the named gates, the local exact-RC acceptance receipt), then creates the annotated tag on the
//! exact release candidate, locally. An existing tag on another commit is a hard failure that needs
//! a human; a tag already on the candidate is success (idempotent). Nothing here moves or deletes a
//! tag, and nothing pushes one.
use super::{blocked, failed, release_dir};
use crate::validation_campaign::{
    executor::{RunContext, RunnerOutcome, TaskRunner},
    graph::{Graph, TaskSpec, validation_gates},
    model::FailureClass,
    shell::Shell,
    verify::verify_state,
};

pub struct TagRunner {
    pub shell: Box<dyn Shell>,
}

/// Gates that must be satisfied before the tag (every one but the release itself).
pub const TAG_GATES: [&str; 7] = [
    "observation",
    "kernel-compatibility",
    "diagnosis-corpus",
    "diagnosis-calibration",
    "score-evaluation",
    "capability-policy",
    "profiler-overhead",
];

impl TaskRunner for TagRunner {
    fn run(&self, context: &RunContext<'_>, _spec: &TaskSpec, _attempt: u32) -> RunnerOutcome {
        let release = &context.manifest.release;
        if !release.auto_tag {
            return blocked(
                FailureClass::BlockedResource,
                "the manifest does not enable automatic tagging",
            );
        }
        let Some(commit) = context.state.release_candidate.clone() else {
            return blocked(
                FailureClass::BlockedResource,
                "no release candidate has been frozen",
            );
        };
        let Ok(graph) = Graph::core() else {
            return failed(
                FailureClass::SoftwareDefect,
                "graph",
                "the task graph is invalid",
            );
        };
        let problems = verify_state(context.dir, context.state, &graph, context.manifest);
        if !problems.is_empty() {
            return failed(
                FailureClass::SoftwareDefect,
                "verify",
                format!("strict verification fails: {}", problems.join("; ")),
            );
        }
        let open: Vec<String> = validation_gates()
            .into_iter()
            .filter(|gate| TAG_GATES.contains(&gate.name.as_str()))
            .filter(|gate| {
                gate.tasks
                    .iter()
                    .filter(|task| task.as_str() != "tag")
                    .any(|task| {
                        graph.get(task).is_none_or(|spec| {
                            !context.state.effective_status(spec).satisfies_dependents()
                        })
                    })
            })
            .map(|gate| gate.name)
            .collect();
        if !open.is_empty() {
            return failed(
                FailureClass::SoftwareDefect,
                "gates",
                format!("validation gates {open:?} are not satisfied"),
            );
        }
        if let Err(problem) = acceptance_receipt_problem(context, &commit) {
            return failed(FailureClass::SoftwareDefect, "acceptance", problem);
        }
        create_tag(self.shell.as_ref(), context.repository, release, &commit)
    }
}

/// The exact-RC acceptance receipt of this candidate must exist and be a fresh-checkout PASS.
pub fn acceptance_receipt_problem(context: &RunContext<'_>, commit: &str) -> Result<(), String> {
    let dir = release_dir(&context.dir.0).map_err(|e| e.to_string())?;
    let text = std::fs::read(dir.join("offline-acceptance.json"))
        .map_err(|e| format!("no exact-RC acceptance receipt: {e}"))?;
    let receipt: serde_json::Value = serde_json::from_slice(&text)
        .map_err(|e| format!("the acceptance receipt is unreadable: {e}"))?;
    if receipt["status"] != "PASS"
        || receipt["commit"] != commit
        || receipt["fresh_checkout"] != true
    {
        return Err(format!(
            "the acceptance receipt is not a fresh-checkout PASS of {commit}"
        ));
    }
    Ok(())
}

/// Everything after the campaign-state checks: the repository must be exactly the candidate and
/// the annotated tag is created locally, only if it does not exist yet. Nothing here talks to a
/// remote or moves an existing tag; publishing a valid local tag is outside verification.
pub fn create_tag(
    shell: &dyn Shell,
    repo: &std::path::Path,
    release: &crate::validation_campaign::manifest::ReleaseSection,
    commit: &str,
) -> RunnerOutcome {
    let git = |args: &[&str]| shell.run("git", args, repo);

    if git(&["rev-parse", "HEAD"]).stdout.trim() != commit {
        return failed(
            FailureClass::SoftwareDefect,
            "head",
            "HEAD is not the release candidate",
        );
    }
    if !git(&["status", "--porcelain"]).stdout.trim().is_empty() {
        return failed(
            FailureClass::SoftwareDefect,
            "clean",
            "the working tree is dirty",
        );
    }
    let tag = release.tag.as_str();
    let existing = git(&[
        "rev-parse",
        "-q",
        "--verify",
        &format!("refs/tags/{tag}^{{commit}}"),
    ]);
    if existing.ok() {
        return if existing.stdout.trim() == commit {
            RunnerOutcome::passed()
        } else {
            failed(
                FailureClass::ValidationFailure,
                "tag-exists",
                format!(
                    "FAILURE_REQUIRES_REVIEW: {tag} already points at {}, not the release candidate {commit}; it is never moved",
                    existing.stdout.trim()
                ),
            )
        };
    }
    let created = git(&[
        "tag",
        "-a",
        tag,
        commit,
        "-m",
        &format!("Stutter v{}", release.version),
    ]);
    if !created.ok() {
        return failed(
            FailureClass::Retryable,
            "tag",
            created.stderr.trim().to_owned(),
        );
    }
    RunnerOutcome::passed()
}
