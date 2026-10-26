//! The local landing primitive, against real git repositories: no remote exists in any of them.
use std::{fs, path::Path, process::Command};

use super::Scratch;
use crate::validation_campaign::{
    landing::{LandError, Landed, LocalChange, land},
    shell::SystemShell,
};

fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {output:?}");
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// A repository with one commit on `main` and no remote.
fn repo(name: &str) -> (Scratch, std::path::PathBuf) {
    let scratch = Scratch::new(name);
    let path = scratch.path().to_owned();
    git(&path, &["init", "-q", "-b", "main"]);
    git(&path, &["config", "user.email", "t@example.test"]);
    git(&path, &["config", "user.name", "t"]);
    git(&path, &["config", "commit.gpgsign", "false"]);
    fs::write(path.join("file.txt"), "one\n").unwrap();
    git(&path, &["add", "-A"]);
    git(&path, &["commit", "-q", "-m", "base"]);
    (scratch, path)
}

const BRANCH: &str = "campaign/x/integration";

fn change() -> LocalChange<'static> {
    LocalChange {
        branch: BRANCH,
        commit_message: "land it",
    }
}

fn write_file(path: &Path) -> impl Fn() -> Result<(), String> + '_ {
    move || fs::write(path.join("file.txt"), "two\n").map_err(|e| e.to_string())
}

#[test]
fn a_verified_change_fast_forwards_main_and_removes_its_branch() {
    let (_scratch, path) = repo("landing-ok");
    let base = git(&path, &["rev-parse", "main"]);
    let verified = std::cell::Cell::new(false);
    let landed = land(&SystemShell, &path, &change(), &write_file(&path), &|| {
        // Verification runs on the task branch, before main has moved.
        assert_eq!(git(&path, &["rev-parse", "--abbrev-ref", "HEAD"]), BRANCH);
        assert_eq!(git(&path, &["rev-parse", "main"]), base);
        verified.set(true);
        Ok(())
    })
    .unwrap();
    assert!(verified.get());
    let Landed::Merged { commit } = landed else {
        panic!("expected a merge")
    };
    assert_eq!(commit, git(&path, &["rev-parse", "main"]));
    assert_ne!(commit, base);
    assert_eq!(git(&path, &["rev-parse", "--abbrev-ref", "HEAD"]), "main");
    assert_eq!(fs::read_to_string(path.join("file.txt")).unwrap(), "two\n");
    assert!(git(&path, &["branch", "--list", BRANCH]).is_empty());
    assert_eq!(git(&path, &["log", "--format=%s", "-1"]), "land it");
}

#[test]
fn a_change_that_does_not_verify_never_reaches_main() {
    let (_scratch, path) = repo("landing-red");
    let base = git(&path, &["rev-parse", "main"]);
    let error = land(&SystemShell, &path, &change(), &write_file(&path), &|| {
        Err("clippy: warnings".to_owned())
    })
    .unwrap_err();
    assert!(matches!(
        error,
        LandError::Failed { ref stage, retryable: false, .. } if stage == "verify"
    ));
    assert_eq!(git(&path, &["rev-parse", "main"]), base);
    assert_eq!(fs::read_to_string(path.join("file.txt")).unwrap(), "one\n");
    // The branch stays for inspection and the checkout is back on main.
    assert!(!git(&path, &["branch", "--list", BRANCH]).is_empty());
    assert_eq!(git(&path, &["rev-parse", "--abbrev-ref", "HEAD"]), "main");
}

#[test]
fn a_change_that_changes_nothing_lands_nothing() {
    let (_scratch, path) = repo("landing-nochange");
    let base = git(&path, &["rev-parse", "main"]);
    let landed = land(&SystemShell, &path, &change(), &|| Ok(()), &|| Ok(())).unwrap();
    assert_eq!(landed, Landed::NoChange);
    assert_eq!(git(&path, &["rev-parse", "main"]), base);
    assert!(git(&path, &["branch", "--list", BRANCH]).is_empty());
}

#[test]
fn a_dirty_checkout_is_refused_before_anything_is_touched() {
    let (_scratch, path) = repo("landing-dirty");
    fs::write(path.join("file.txt"), "local edit\n").unwrap();
    let error = land(&SystemShell, &path, &change(), &write_file(&path), &|| {
        Ok(())
    })
    .unwrap_err();
    assert!(matches!(error, LandError::Blocked(_)));
    assert_eq!(
        fs::read_to_string(path.join("file.txt")).unwrap(),
        "local edit\n"
    );
}

#[test]
fn an_interrupted_landing_resumes_from_its_branch_without_applying_twice() {
    let (_scratch, path) = repo("landing-resume");
    // A first attempt committed on the branch and was interrupted before verifying.
    git(&path, &["checkout", "-q", "-b", BRANCH]);
    fs::write(path.join("file.txt"), "two\n").unwrap();
    git(&path, &["commit", "-q", "-am", "land it"]);
    git(&path, &["checkout", "-q", "main"]);
    let applied = std::cell::Cell::new(false);
    let landed = land(
        &SystemShell,
        &path,
        &change(),
        &|| {
            applied.set(true);
            Ok(())
        },
        &|| Ok(()),
    )
    .unwrap();
    assert!(!applied.get(), "the existing branch is reused");
    assert!(matches!(landed, Landed::Merged { .. }));
    assert_eq!(fs::read_to_string(path.join("file.txt")).unwrap(), "two\n");
}

#[test]
fn a_change_already_on_main_is_recognised_after_an_interruption() {
    let (_scratch, path) = repo("landing-done");
    git(&path, &["branch", BRANCH]);
    let landed = land(
        &SystemShell,
        &path,
        &change(),
        &|| Err("no".to_owned()),
        &|| Ok(()),
    )
    .unwrap();
    assert!(matches!(landed, Landed::Merged { .. }));
    assert!(git(&path, &["branch", "--list", BRANCH]).is_empty());
}

#[test]
fn a_branch_made_on_a_stale_main_is_redone_on_the_new_base() {
    let (_scratch, path) = repo("landing-stale");
    git(&path, &["checkout", "-q", "-b", BRANCH]);
    fs::write(path.join("stale.txt"), "stale\n").unwrap();
    git(&path, &["add", "-A"]);
    git(&path, &["commit", "-q", "-m", "stale change"]);
    git(&path, &["checkout", "-q", "main"]);
    fs::write(path.join("other.txt"), "moved on\n").unwrap();
    git(&path, &["add", "-A"]);
    git(&path, &["commit", "-q", "-m", "main moved"]);
    let landed = land(&SystemShell, &path, &change(), &write_file(&path), &|| {
        Ok(())
    })
    .unwrap();
    assert!(matches!(landed, Landed::Merged { .. }));
    assert!(path.join("other.txt").is_file());
    assert!(
        !path.join("stale.txt").exists(),
        "the stale change was regenerated, not merged"
    );
    assert_eq!(fs::read_to_string(path.join("file.txt")).unwrap(), "two\n");
}

#[test]
fn main_moving_while_verifying_is_a_retryable_failure_and_main_is_untouched() {
    let (_scratch, path) = repo("landing-race");
    let error = land(&SystemShell, &path, &change(), &write_file(&path), &|| {
        git(&path, &["checkout", "-q", "main"]);
        fs::write(path.join("other.txt"), "x\n").unwrap();
        git(&path, &["add", "-A"]);
        git(&path, &["commit", "-q", "-m", "someone else"]);
        git(&path, &["checkout", "-q", BRANCH]);
        Ok(())
    })
    .unwrap_err();
    assert!(matches!(
        error,
        LandError::Failed { ref stage, retryable: true, .. } if stage == "main-moved"
    ));
    assert_eq!(
        git(&path, &["log", "--format=%s", "-1", "main"]),
        "someone else"
    );
}

#[test]
fn landing_never_touches_a_remote_or_a_hosted_cli() {
    // The primitive only ever runs git on the local repository; this repository has no remote at
    // all, so any push, pull or fetch would have failed the tests above.
    let (_scratch, path) = repo("landing-no-remote");
    assert!(git(&path, &["remote"]).is_empty());
    let landed = land(&SystemShell, &path, &change(), &write_file(&path), &|| {
        Ok(())
    })
    .unwrap();
    assert!(matches!(landed, Landed::Merged { .. }));
}
