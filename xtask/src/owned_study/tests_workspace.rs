//! E10: cleanup only ever deletes what the study created and can prove it owns.
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::Command,
};

use stutter::api::workload_progress::FixedWorkIdentity;

use super::{
    tests::scratch,
    workspace::{Workspace, prepare},
};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-C",
        ])
        .arg(dir)
        .args(args)
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// A committed cargo tree and its identity; the tree has a sentinel `target` like a user's project.
fn source(name: &str) -> (PathBuf, FixedWorkIdentity) {
    let dir = fs::canonicalize(scratch(name)).expect("canonical");
    let tree = dir.join("src");
    fs::create_dir_all(&tree).expect("src");
    fs::write(
        tree.join("Cargo.toml"),
        "[package]\nname=\"x\"\nversion=\"0.1.0\"\n",
    )
    .expect("manifest");
    fs::write(tree.join(".gitignore"), "target\n").expect("ignore");
    git(&tree, &["init", "-q"]);
    git(&tree, &["add", "-A"]);
    git(&tree, &["commit", "-q", "-m", "x"]);
    let hash = git(&tree, &["rev-parse", "HEAD^{tree}"]);
    let identity = FixedWorkIdentity {
        kind: serde_json::from_str("\"compile\"").expect("kind"),
        source_tree: format!("git:{hash}"),
        target: "t".into(),
        build_state: "cold".into(),
        cache_policy: "none".into(),
        job_count: 2,
        command: vec!["cargo".into(), "build".into()],
        working_directory: tree.display().to_string(),
        toolchain: "t".into(),
        environment: "e".into(),
    };
    (dir, identity)
}

fn study_dir(base: &Path, name: &str) -> PathBuf {
    let out = base.join(name);
    fs::create_dir(&out).expect("study dir");
    fs::set_permissions(&out, fs::Permissions::from_mode(0o700)).expect("0700");
    out
}

fn ready(name: &str) -> (PathBuf, FixedWorkIdentity, Workspace) {
    let (base, identity) = source(name);
    let out = study_dir(&base, "study");
    let ws = prepare(&out, "demo", "digest", &identity).expect("workspace");
    (out, identity, ws)
}

#[test]
fn a_users_existing_target_directory_is_never_touched() {
    let (base, identity) = source("ws-user-target");
    let user_target = base.join("src/target");
    fs::create_dir_all(&user_target).expect("user target");
    fs::write(user_target.join("sentinel"), "mine").expect("sentinel");
    let out = study_dir(&base, "study");
    let ws = prepare(&out, "demo", "digest", &identity).expect("workspace");
    fs::create_dir_all(ws.target_dir().join("debug")).expect("owned build output");
    ws.reset_target().expect("reset");
    assert!(
        !ws.target_dir().exists(),
        "the study's own target is removed"
    );
    assert_eq!(
        fs::read_to_string(user_target.join("sentinel")).expect("sentinel"),
        "mine"
    );
    ws.reset_target().expect("idempotent when nothing is there");
}

#[test]
fn a_symlinked_target_or_workspace_is_refused_and_its_destination_survives() {
    let (out, _, ws) = ready("ws-symlink");
    let victim = out.parent().expect("base").join("victim");
    fs::create_dir(&victim).expect("victim");
    fs::write(victim.join("keep"), "x").expect("keep");
    symlink(&victim, ws.target_dir()).expect("symlink");
    let err = ws.reset_target().expect_err("symlink");
    assert!(err.0.contains("symlink"), "{err}");
    assert!(victim.join("keep").exists());
    fs::remove_file(ws.target_dir()).expect("rm link");
    let root = out.join("workspace");
    fs::rename(&root, out.join("moved")).expect("move");
    symlink(&victim, &root).expect("workspace symlink");
    assert!(ws.reset_target().is_err());
    assert!(victim.join("keep").exists());
}

#[test]
fn a_path_alias_to_the_source_is_refused() {
    let (base, mut identity) = source("ws-alias");
    let alias = base.join("alias");
    symlink(base.join("src"), &alias).expect("alias");
    identity.working_directory = alias.display().to_string();
    let out = study_dir(&base, "study");
    assert!(
        prepare(&out, "demo", "d", &identity)
            .expect_err("alias")
            .0
            .contains("symlink")
    );
    identity.working_directory = "relative/src".into();
    assert!(prepare(&out, "demo", "d", &identity).is_err());
}

#[test]
fn another_studys_workspace_and_a_missing_marker_are_refused() {
    let (base, identity) = source("ws-marker");
    let out = study_dir(&base, "study");
    prepare(&out, "demo", "digest-a", &identity).expect("first");
    assert!(
        prepare(&out, "demo", "digest-b", &identity)
            .expect_err("other manifest")
            .0
            .contains("marker")
    );
    assert!(
        prepare(&out, "other", "digest-a", &identity).is_err(),
        "other study id"
    );
    prepare(&out, "demo", "digest-a", &identity).expect("the owner may resume");
    fs::remove_file(out.join("workspace/.owned-study")).expect("rm marker");
    assert!(
        prepare(&out, "demo", "digest-a", &identity).is_err(),
        "no marker means not owned"
    );
}

#[test]
fn a_study_directory_that_is_not_private_is_refused() {
    let (base, identity) = source("ws-mode");
    let out = study_dir(&base, "study");
    fs::set_permissions(&out, fs::Permissions::from_mode(0o755)).expect("chmod");
    assert!(prepare(&out, "demo", "d", &identity).is_err());
    assert!(prepare(&base.join("missing"), "demo", "d", &identity).is_err());
}

#[test]
fn a_source_that_is_not_the_frozen_clean_tree_is_refused() {
    let (base, identity) = source("ws-tree");
    let out = study_dir(&base, "study");
    let tree = base.join("src");
    fs::write(
        tree.join("Cargo.toml"),
        "[package]\nname=\"changed\"\nversion=\"0.1.0\"\n",
    )
    .expect("edit");
    assert!(
        prepare(&out, "demo", "d", &identity)
            .expect_err("dirty")
            .0
            .contains("modified")
    );
    git(&tree, &["checkout", "--", "Cargo.toml"]);
    let mut wrong = identity.clone();
    wrong.source_tree = format!("git:{}", "0".repeat(40));
    assert!(
        prepare(&out, "demo", "d", &wrong)
            .expect_err("tree")
            .0
            .contains("frozen")
    );
    wrong.source_tree = format!("sha256:{}", "0".repeat(64));
    assert!(
        prepare(&out, "demo", "d", &wrong).is_err(),
        "only a git tree can be verified"
    );
    prepare(&out, "demo", "d", &identity).expect("the frozen tree is accepted");
}

#[test]
fn the_study_directory_may_not_overlap_the_source() {
    let (base, identity) = source("ws-overlap");
    let inside = study_dir(&base.join("src"), "study");
    assert!(prepare(&inside, "demo", "d", &identity).is_err());
}

#[test]
fn a_build_that_modified_the_source_is_detected_after_the_slot() {
    let (out, identity, ws) = ready("ws-modified");
    ws.verify_unchanged_source().expect("clean");
    fs::write(
        Path::new(&identity.working_directory).join("Cargo.toml"),
        "[package]\nname=\"x2\"\nversion=\"0.1.0\"\n",
    )
    .expect("edit");
    assert!(ws.verify_unchanged_source().is_err());
    assert!(
        ws.reset_target().is_ok(),
        "the owned target can still be reset"
    );
    let _ = out;
}

#[test]
fn a_target_that_is_not_directly_inside_the_workspace_is_never_removed() {
    let (out, _, ws) = ready("ws-escape");
    let elsewhere = out.parent().expect("base").join("elsewhere");
    fs::create_dir_all(elsewhere.join("t")).expect("elsewhere");
    fs::write(elsewhere.join("t/keep"), "x").expect("keep");
    symlink(elsewhere.join("t"), ws.target_dir()).expect("link");
    assert!(ws.reset_target().is_err());
    assert!(elsewhere.join("t/keep").exists());
}
