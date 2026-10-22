use std::{fs, path::Path, process::Command};

use super::{MANIFEST_TEXT, Scratch};
use crate::validation_campaign::init;

fn git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(repo)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

/// A committed repository on branch `main` holding the campaign manifest.
fn repository(scratch: &Scratch, branch: &str) {
    let repo = scratch.path();
    git(repo, &["init", "-q", "-b", branch]);
    git(repo, &["config", "user.email", "t@example.org"]);
    git(repo, &["config", "user.name", "t"]);
    fs::create_dir_all(repo.join("docs/validation")).unwrap();
    fs::write(
        repo.join("docs/validation/core-validation.toml"),
        MANIFEST_TEXT,
    )
    .unwrap();
    fs::write(repo.join("Cargo.lock"), "# lock\n").unwrap();
    fs::write(repo.join(".gitignore"), "target/\n.llm-runs/\n").unwrap();
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-q", "-m", "base"]);
}

fn try_init(scratch: &Scratch) -> anyhow::Result<()> {
    init(
        scratch.path(),
        Path::new("docs/validation/core-validation.toml"),
    )
}

#[test]
fn init_refuses_a_dirty_working_tree() {
    let scratch = Scratch::new("init-dirty");
    repository(&scratch, "main");
    fs::write(scratch.path().join("untracked"), "x").unwrap();
    let error = format!("{:#}", try_init(&scratch).unwrap_err());
    assert!(error.contains("dirty"), "{error}");
}

#[test]
fn init_refuses_to_start_from_a_branch_other_than_main() {
    let scratch = Scratch::new("init-branch");
    repository(&scratch, "feature");
    let error = format!("{:#}", try_init(&scratch).unwrap_err());
    assert!(error.contains("clean `main`"), "{error}");
}

#[test]
fn init_requires_a_stutter_binary_matching_the_source() {
    let scratch = Scratch::new("init-binary");
    repository(&scratch, "main");
    let error = format!("{:#}", try_init(&scratch).unwrap_err());
    assert!(error.contains("target/debug/stutter"), "{error}");
}

#[test]
fn init_rejects_an_invalid_manifest_and_one_outside_the_repository() {
    let scratch = Scratch::new("init-manifest");
    repository(&scratch, "main");
    let bad = scratch.path().join("docs/validation/core-validation.toml");
    fs::write(&bad, MANIFEST_TEXT.replace("v0.1.0", "v9.9.9")).unwrap();
    assert!(try_init(&scratch).is_err());
    let outside = Scratch::new("init-outside");
    fs::write(outside.path().join("m.toml"), MANIFEST_TEXT).unwrap();
    let error = format!(
        "{:#}",
        init(scratch.path(), &outside.path().join("m.toml")).unwrap_err()
    );
    assert!(error.contains("inside the repository"), "{error}");
}

#[test]
fn the_cli_lifecycle_init_status_next_verify_report_and_cancel() {
    use crate::validation_campaign::{
        cli::{CampaignArgs, CampaignCommand},
        init_with, load_campaign,
        model::CampaignMode,
        run,
    };
    let scratch = Scratch::new("lifecycle");
    repository(&scratch, "main");
    fs::create_dir_all(scratch.path().join("target/debug")).unwrap();
    fs::write(scratch.path().join("target/debug/stutter"), "stub").unwrap();
    // The stub cannot report a real revision; the injected check stands in for that rule.
    init_with(
        scratch.path(),
        Path::new("docs/validation/core-validation.toml"),
        &|_, _| Ok(()),
    )
    .unwrap();
    let loaded = load_campaign(scratch.path(), None).unwrap();
    assert_eq!(loaded.state.mode, CampaignMode::Collector);
    assert_eq!(loaded.state.tasks.len(), 24);
    assert_eq!(loaded.state.ready(&loaded.graph), ["offline-ready"]);
    // A second init is refused while this campaign is active.
    let again = init_with(
        scratch.path(),
        Path::new("docs/validation/core-validation.toml"),
        &|_, _| Ok(()),
    );
    assert!(format!("{:#}", again.unwrap_err()).contains("still active"));
    let command = |command| run(scratch.path(), CampaignArgs { command });
    command(CampaignCommand::Status {
        json: true,
        campaign: None,
    })
    .unwrap();
    command(CampaignCommand::Next { campaign: None }).unwrap();
    command(CampaignCommand::Verify {
        campaign: None,
        strict: true,
    })
    .unwrap();
    command(CampaignCommand::Report { campaign: None }).unwrap();
    assert!(loaded.dir.0.join("report.json").is_file() && loaded.dir.0.join("report.md").is_file());
    // Cancel stops automation but deletes nothing.
    command(CampaignCommand::Cancel { campaign: None }).unwrap();
    let cancelled = load_campaign(scratch.path(), None).unwrap();
    assert_eq!(cancelled.state.mode, CampaignMode::Invalidated);
    assert!(loaded.dir.state_path().is_file() && loaded.dir.manifest_snapshot().is_file());
    // With the campaign no longer active, a successor may be initialized.
    init_with(
        scratch.path(),
        Path::new("docs/validation/core-validation.toml"),
        &|_, _| Ok(()),
    )
    .unwrap();
}
