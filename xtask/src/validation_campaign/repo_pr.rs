//! Lands a repository change the way the plan requires: branch from a clean `main`, commit, push,
//! open a PR, wait for the required checks, merge, return to clean `main`. Every step goes through
//! a [`Shell`]; an interrupted flow resumes from the branch's PR instead of starting over, and a
//! failed or pending check is never merged over.
use std::{path::Path, time::Duration};

use serde_json::Value;

use super::shell::Shell;

/// Checks that never gate a merge: the hardware job has no hardware on GitHub's runners.
pub const IGNORED_CHECKS: &[&str] = &["live smoke tests"];

pub struct PrChange<'a> {
    pub branch: &'a str,
    pub title: &'a str,
    pub body: &'a str,
    pub commit_message: &'a str,
}

pub struct WaitPolicy {
    pub poll: Duration,
    pub max_polls: u32,
}

impl Default for WaitPolicy {
    fn default() -> Self {
        Self {
            poll: Duration::from_secs(30),
            max_polls: 120,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum Landed {
    /// The change was already part of `main`; nothing was opened.
    NoChange,
    Merged {
        commit: String,
    },
}

#[derive(Debug, Eq, PartialEq)]
pub enum PrError {
    /// Needs something the machine does not have (an authenticated `gh`, a clean checkout).
    Blocked(String),
    /// A step failed; `retryable` is false when the code under review is at fault (red CI).
    Failed {
        stage: String,
        detail: String,
        retryable: bool,
    },
}

fn blocked(detail: impl Into<String>) -> PrError {
    PrError::Blocked(detail.into())
}

fn failed(stage: &str, detail: impl Into<String>, retryable: bool) -> PrError {
    PrError::Failed {
        stage: stage.to_owned(),
        detail: detail.into(),
        retryable,
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum Checks {
    Pending,
    Green,
    Red(Vec<String>),
}

/// Classifies `statusCheckRollup`: anything not conclusively successful is not green. Skipped,
/// cancelled and neutral required checks block exactly like failures.
pub fn classify_checks(rollup: &Value) -> Checks {
    let entries = rollup.as_array().cloned().unwrap_or_default();
    let (mut pending, mut red, mut counted) = (false, Vec::new(), 0);
    for entry in &entries {
        let name = entry["name"]
            .as_str()
            .or_else(|| entry["context"].as_str())
            .unwrap_or("unnamed");
        if IGNORED_CHECKS.contains(&name) {
            continue;
        }
        counted += 1;
        let conclusion = entry["conclusion"]
            .as_str()
            .or_else(|| entry["state"].as_str())
            .unwrap_or("");
        match conclusion {
            "SUCCESS" => {}
            "" | "PENDING" | "EXPECTED" => pending = true,
            _ if entry["status"].as_str().is_some_and(|s| s != "COMPLETED") => pending = true,
            _ => red.push(format!("{name}={conclusion}")),
        }
    }
    if !red.is_empty() {
        Checks::Red(red)
    } else if pending || counted == 0 {
        Checks::Pending
    } else {
        Checks::Green
    }
}

fn git(shell: &dyn Shell, repo: &Path, args: &[&str], stage: &str) -> Result<String, PrError> {
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

fn sync_main(shell: &dyn Shell, repo: &Path) -> Result<String, PrError> {
    git(shell, repo, &["checkout", "main"], "checkout")?;
    git(shell, repo, &["pull", "--ff-only"], "pull")?;
    Ok(git(shell, repo, &["rev-parse", "HEAD"], "rev-parse")?
        .trim()
        .to_owned())
}

/// `apply` writes the change into the working tree (it runs on the new branch).
pub fn land(
    shell: &dyn Shell,
    repo: &Path,
    change: &PrChange<'_>,
    apply: &dyn Fn() -> Result<(), String>,
    wait: &WaitPolicy,
) -> Result<Landed, PrError> {
    if !shell.run("gh", &["auth", "status"], repo).ok() {
        return Err(blocked(
            "`gh auth status` failed: the campaign cannot open or merge pull requests",
        ));
    }
    let existing = shell.run(
        "gh",
        &["pr", "view", change.branch, "--json", "number,state,url"],
        repo,
    );
    let pr = if existing.ok() {
        serde_json::from_str::<Value>(&existing.stdout).ok()
    } else {
        None
    };
    let state = pr
        .as_ref()
        .and_then(|v| v["state"].as_str())
        .unwrap_or("")
        .to_owned();
    if state == "MERGED" {
        sync_main(shell, repo)?;
        return Ok(Landed::Merged {
            commit: git(shell, repo, &["rev-parse", "HEAD"], "rev-parse")?
                .trim()
                .to_owned(),
        });
    }
    let target = if state == "OPEN" {
        pr.as_ref()
            .and_then(|v| v["url"].as_str())
            .unwrap_or(change.branch)
            .to_owned()
    } else {
        if !git(shell, repo, &["status", "--porcelain"], "status")?
            .trim()
            .is_empty()
        {
            return Err(blocked(
                "the checkout is dirty: refusing to open a pull request over local changes",
            ));
        }
        sync_main(shell, repo)?;
        git(shell, repo, &["checkout", "-B", change.branch], "branch")?;
        apply().map_err(|e| failed("apply", e, false))?;
        git(shell, repo, &["add", "-A"], "add")?;
        if shell
            .run("git", &["diff", "--cached", "--quiet"], repo)
            .ok()
        {
            sync_main(shell, repo)?;
            let _ = shell.run("git", &["branch", "-D", change.branch], repo);
            return Ok(Landed::NoChange);
        }
        git(
            shell,
            repo,
            &["commit", "-q", "-m", change.commit_message],
            "commit",
        )?;
        git(
            shell,
            repo,
            &["push", "-q", "-u", "origin", change.branch],
            "push",
        )?;
        let created = shell.run(
            "gh",
            &[
                "pr",
                "create",
                "--base",
                "main",
                "--head",
                change.branch,
                "--title",
                change.title,
                "--body",
                change.body,
            ],
            repo,
        );
        if !created.ok() {
            return Err(failed("pr-create", created.stderr.trim().to_owned(), true));
        }
        created
            .stdout
            .lines()
            .last()
            .unwrap_or(change.branch)
            .trim()
            .to_owned()
    };
    wait_for_green(shell, repo, &target, wait)?;
    let merged = shell.run("gh", &["pr", "merge", &target, "--merge"], repo);
    if !merged.ok() {
        return Err(failed("merge", merged.stderr.trim().to_owned(), true));
    }
    let commit = sync_main(shell, repo)?;
    let _ = shell.run("git", &["branch", "-D", change.branch], repo);
    Ok(Landed::Merged { commit })
}

/// Polls until every counted check is green; red checks stop immediately.
pub fn wait_for_green(
    shell: &dyn Shell,
    repo: &Path,
    target: &str,
    wait: &WaitPolicy,
) -> Result<(), PrError> {
    for _ in 0..wait.max_polls {
        let view = shell.run(
            "gh",
            &["pr", "view", target, "--json", "statusCheckRollup"],
            repo,
        );
        if view.ok() {
            let value: Value = serde_json::from_str(&view.stdout).unwrap_or(Value::Null);
            match classify_checks(&value["statusCheckRollup"]) {
                Checks::Green => return Ok(()),
                Checks::Red(names) => {
                    return Err(failed(
                        "ci",
                        format!("required checks failed: {}", names.join(", ")),
                        false,
                    ));
                }
                Checks::Pending => {}
            }
        }
        shell.sleep(wait.poll);
    }
    Err(failed("ci", "required checks did not finish in time", true))
}
