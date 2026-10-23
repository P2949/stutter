//! `tag`: the last task. It re-checks everything the policy demands, then creates the annotated
//! tag on the exact release candidate. An existing tag on another commit is a hard failure that
//! needs a human; a tag already on the candidate is success (idempotent). Nothing here moves,
//! deletes or force-pushes a tag.
use super::{blocked, ci_must_be_green, failed};
use crate::validation_campaign::{
    executor::{RunContext, RunnerOutcome, TaskRunner},
    graph::{Graph, TaskSpec, issue_gates},
    model::FailureClass,
    repo_pr::WaitPolicy,
    shell::Shell,
    verify::verify_state,
};

pub struct TagRunner {
    pub shell: Box<dyn Shell>,
    pub wait: WaitPolicy,
}

/// Issues whose gates must be satisfied before the tag.
pub const TAG_ISSUES: [u32; 7] = [27, 30, 31, 32, 33, 35, 38];

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
        let open: Vec<u32> = issue_gates()
            .into_iter()
            .filter(|gate| TAG_ISSUES.contains(&gate.issue))
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
            .map(|gate| gate.issue)
            .collect();
        if !open.is_empty() {
            return failed(
                FailureClass::SoftwareDefect,
                "gates",
                format!("issue gates {open:?} are not satisfied"),
            );
        }
        create_tag(
            self.shell.as_ref(),
            context.repository,
            release,
            &commit,
            &self.wait,
        )
    }
}

/// Everything after the campaign-state checks: the repository must be exactly the candidate,
/// CI must be green, and the tag is created only if it does not exist yet.
pub fn create_tag(
    shell: &dyn Shell,
    repo: &std::path::Path,
    release: &crate::validation_campaign::manifest::ReleaseSection,
    commit: &str,
    wait: &WaitPolicy,
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
    if let Err(outcome) = ci_must_be_green(shell, repo, commit, wait) {
        return outcome;
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
            // Created by an earlier, interrupted attempt: make sure origin has it too.
            let remote = git(&["ls-remote", "--tags", "origin", &format!("refs/tags/{tag}")]);
            if remote.ok() && remote.stdout.trim().is_empty() {
                let pushed = git(&["push", "origin", tag]);
                if !pushed.ok() {
                    return failed(
                        FailureClass::Retryable,
                        "push",
                        pushed.stderr.trim().to_owned(),
                    );
                }
            }
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
    let remote = git(&["ls-remote", "--tags", "origin", &format!("refs/tags/{tag}")]);
    if remote.ok() && !remote.stdout.trim().is_empty() {
        return failed(
            FailureClass::ValidationFailure,
            "tag-exists",
            format!("FAILURE_REQUIRES_REVIEW: {tag} already exists on origin; it is never moved"),
        );
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
    let pushed = git(&["push", "origin", tag]);
    if !pushed.ok() {
        return failed(
            FailureClass::Retryable,
            "push",
            pushed.stderr.trim().to_owned(),
        );
    }
    RunnerOutcome::passed()
}
