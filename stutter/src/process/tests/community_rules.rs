//! Tests for process-tree integration with community-rule classification.
//!
//! Owns process scanner community-rule regression tests. Does not own production procfs scanning,
//! caching, classification, or tree rendering.

use std::{fs, os::unix::fs::symlink, path::Path};

use tempfile::tempdir;

use super::*;
use crate::community_rules::{
    CommunityRule, CommunityRulesDb, CommunityRulesFile, CommunityRulesSource,
};

fn write_fake_proc_task(proc_root: &Path, pid: u32, comm: &str, exe_path: &str) {
    let proc_dir = proc_root.join(pid.to_string());
    let task_dir = proc_dir.join("task").join(pid.to_string());

    fs::create_dir_all(&task_dir).unwrap();
    fs::write(
        proc_dir.join("status"),
        format!("Name:\t{comm}\nPPid:\t1\n"),
    )
    .unwrap();
    fs::write(proc_dir.join("cmdline"), format!("{exe_path}\0--test\0")).unwrap();
    fs::write(
        proc_dir.join("cgroup"),
        "0::/user.slice/app-mystery-123.scope\n",
    )
    .unwrap();
    fs::write(task_dir.join("comm"), format!("{comm}\n")).unwrap();
    fs::write(
        proc_dir.join("stat"),
        format!(
            "{pid} ({comm}) S 1 0 0 0 0 0 0 0 0 0 0 0 0 0 20 0 1 0 12345 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n"
        ),
    )
    .unwrap();

    let _ = fs::remove_file(proc_dir.join("exe"));
    symlink(exe_path, proc_dir.join("exe")).unwrap();
}

fn community_rules_db_for_exe(name: &str) -> CommunityRulesDb {
    CommunityRulesDb::from_file(CommunityRulesFile {
        schema_version: 1,
        source: CommunityRulesSource {
            name: "test community rules".to_owned(),
            repo: None,
            commit: None,
            generated_at: "2026-05-09T00:00:00Z".to_owned(),
        },
        rules: vec![CommunityRule {
            name: name.to_owned(),
            normalized_name: name.to_ascii_lowercase(),
            r#type: "Game".to_owned(),
            stutter_class: "Game".to_owned(),
            confidence: 0.90,
            source_path: "test.rules".to_owned(),
            context: vec!["none".to_owned()],
            title: Some("Community Rule Test Game".to_owned()),
            source_url: None,
            comment: None,
            ambiguous: false,
        }],
    })
    .unwrap()
}

#[test]
fn validation_corpus_community_rules_classifies_unknown_game() {
    let dir = tempdir().unwrap();
    let proc_root = dir.path();
    let unknown_pid = 6101;
    let already_classified_pid = 6102;
    let manual_pids = vec![unknown_pid, already_classified_pid];

    write_fake_proc_task(proc_root, unknown_pid, "mysteryproc", "/tmp/community-game");
    write_fake_proc_task(
        proc_root,
        already_classified_pid,
        "wineserver",
        "/tmp/community-game",
    );

    let without_rules = target_snapshot(
        TargetSnapshotInput::default()
            .proc_root(proc_root)
            .manual_pids(&manual_pids),
    );

    assert_eq!(
        without_rules.tasks.get(&unknown_pid).map(|task| task.class),
        Some(TaskClass::Unknown)
    );
    assert_eq!(
        without_rules
            .tasks
            .get(&already_classified_pid)
            .map(|task| task.class),
        Some(TaskClass::WineServer)
    );

    let db = community_rules_db_for_exe("community-game");
    let with_rules = target_snapshot(
        TargetSnapshotInput::default()
            .proc_root(proc_root)
            .manual_pids(&manual_pids)
            .community_rules(Some(&db)),
    );

    assert_eq!(
        with_rules.tasks.get(&unknown_pid).map(|task| task.class),
        Some(TaskClass::Game),
        "community rules should classify locally-Unknown task as Game"
    );
    assert_eq!(
        with_rules
            .tasks
            .get(&already_classified_pid)
            .map(|task| task.class),
        Some(TaskClass::WineServer),
        "community rules must not overwrite tasks already classified by local rules"
    );
}

#[test]
fn target_snapshot_uses_community_rules_only_for_unknown_tasks() {
    let dir = tempdir().unwrap();
    let proc_root = dir.path();
    let pid = 4242;
    let manual_pids = vec![pid];

    write_fake_proc_task(proc_root, pid, "mysteryproc", "/tmp/community-game");

    let without_rules = target_snapshot(
        TargetSnapshotInput::default()
            .proc_root(proc_root)
            .manual_pids(&manual_pids),
    );

    assert_eq!(
        without_rules.tasks.get(&pid).map(|task| task.class),
        Some(TaskClass::Unknown)
    );

    let db = community_rules_db_for_exe("community-game");
    let input_with_rules = TargetSnapshotInput::default()
        .proc_root(proc_root)
        .manual_pids(&manual_pids)
        .community_rules(Some(&db));
    let with_rules = target_snapshot(input_with_rules);

    assert_eq!(
        with_rules.tasks.get(&pid).map(|task| task.class),
        Some(TaskClass::Game)
    );
}
