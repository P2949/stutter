//! Lands a repository change locally and transactionally: record the base `main`, apply the change
//! on an isolated task branch, verify it there, and fast-forward `main` onto it only if `main` is
//! still the base. Every step goes through a [`Shell`]; an interrupted flow resumes from the
//! branch instead of starting over, and a change that does not verify is never landed.
//!
//! Nothing here pushes, opens a pull request, polls a hosted check or talks to a remote: whether a
//! change is good is decided by the local verification the caller supplies.
use std::path::Path;

use super::shell::Shell;

pub struct LocalChange<'a> {
    pub branch: &'a str,
    pub commit_message: &'a str,
}

#[derive(Debug, Eq, PartialEq)]
pub enum Landed {
    /// The change was already part of `main`; nothing was committed.
    NoChange,
    Merged {
        commit: String,
    },
}

#[derive(Debug, Eq, PartialEq)]
pub enum LandError {
    /// Needs something the machine does not have (a clean checkout).
    Blocked(String),
    /// A step failed; `retryable` is false when the change under review is at fault.
    Failed {
        stage: String,
        detail: String,
        retryable: bool,
    },
}

fn blocked(detail: impl Into<String>) -> LandError {
    LandError::Blocked(detail.into())
}

fn failed(stage: &str, detail: impl Into<String>, retryable: bool) -> LandError {
    LandError::Failed {
        stage: stage.to_owned(),
        detail: detail.into(),
        retryable,
    }
}

fn git(shell: &dyn Shell, repo: &Path, args: &[&str], stage: &str) -> Result<String, LandError> {
    let out = shell.run("git", args, repo);
    if out.ok() {
        Ok(out.stdout)
    } else {
        Err(failed(
            stage,
            format!("git {} failed: {}", args.join(" "), out.stderr.trim()),
            true,
        ))
    }
}

fn rev(shell: &dyn Shell, repo: &Path, reference: &str) -> Result<String, LandError> {
    Ok(git(shell, repo, &["rev-parse", reference], "rev-parse")?
        .trim()
        .to_owned())
}

fn is_ancestor(shell: &dyn Shell, repo: &Path, ancestor: &str, of: &str) -> bool {
    shell
        .run("git", &["merge-base", "--is-ancestor", ancestor, of], repo)
        .ok()
}

/// `apply` writes the change into the working tree (it runs on the task branch); `verify` is the
/// local gate the branch must pass before `main` moves.
pub fn land(
    shell: &dyn Shell,
    repo: &Path,
    change: &LocalChange<'_>,
    apply: &dyn Fn() -> Result<(), String>,
    verify: &dyn Fn() -> Result<(), String>,
) -> Result<Landed, LandError> {
    if !git(shell, repo, &["status", "--porcelain"], "status")?
        .trim()
        .is_empty()
    {
        return Err(blocked(
            "the checkout is dirty: refusing to land a change over local modifications",
        ));
    }
    git(shell, repo, &["checkout", "-q", "main"], "checkout")?;
    let base = rev(shell, repo, "main")?;
    let branch_ref = format!("refs/heads/{}", change.branch);
    let exists = shell
        .run(
            "git",
            &["rev-parse", "--verify", "--quiet", &branch_ref],
            repo,
        )
        .ok();
    if exists {
        let tip = rev(shell, repo, &branch_ref)?;
        if is_ancestor(shell, repo, &tip, "main") {
            // An earlier attempt already landed it and was interrupted before cleaning up.
            git(
                shell,
                repo,
                &["branch", "-q", "-D", change.branch],
                "cleanup",
            )?;
            return Ok(Landed::Merged { commit: base });
        }
        if !is_ancestor(shell, repo, "main", &tip) {
            // `main` moved since the branch was made: regenerate the change on the new base.
            git(
                shell,
                repo,
                &["branch", "-q", "-D", change.branch],
                "cleanup",
            )?;
        }
    }
    let resumed = shell
        .run(
            "git",
            &["rev-parse", "--verify", "--quiet", &branch_ref],
            repo,
        )
        .ok();
    git(
        shell,
        repo,
        &[
            "checkout",
            "-q",
            "-B",
            change.branch,
            if resumed { &branch_ref } else { "main" },
        ],
        "branch",
    )?;
    if !resumed {
        apply().map_err(|error| failed("apply", error, false))?;
        git(shell, repo, &["add", "-A"], "add")?;
        if shell
            .run("git", &["diff", "--cached", "--quiet"], repo)
            .ok()
        {
            git(shell, repo, &["checkout", "-q", "main"], "checkout")?;
            git(
                shell,
                repo,
                &["branch", "-q", "-D", change.branch],
                "cleanup",
            )?;
            return Ok(Landed::NoChange);
        }
        git(
            shell,
            repo,
            &["commit", "-q", "-m", change.commit_message],
            "commit",
        )?;
    }
    if let Err(error) = verify() {
        // The branch is kept for inspection; `main` was never touched.
        let _ = shell.run("git", &["checkout", "-q", "main"], repo);
        return Err(failed("verify", error, false));
    }
    git(shell, repo, &["checkout", "-q", "main"], "checkout")?;
    if rev(shell, repo, "main")? != base {
        return Err(failed(
            "main-moved",
            "local main moved while the change was verified; it will be redone on the new base",
            true,
        ));
    }
    git(
        shell,
        repo,
        &["merge", "--ff-only", "-q", change.branch],
        "merge",
    )?;
    let commit = rev(shell, repo, "HEAD")?;
    git(
        shell,
        repo,
        &["branch", "-q", "-d", change.branch],
        "cleanup",
    )?;
    Ok(Landed::Merged { commit })
}
