use std::{
    fs,
    path::{Path, PathBuf},
};

fn crate_src_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn rust_files_under(path: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_rust_files(path, &mut files);
    files.sort();
    files
}

fn collect_rust_files(path: &Path, files: &mut Vec<PathBuf>) {
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };
    if metadata.is_file() {
        if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            files.push(path.to_path_buf());
        }
        return;
    }

    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        collect_rust_files(&entry.path(), files);
    }
}

fn assert_sources_do_not_contain(files: &[PathBuf], forbidden: &[&str]) {
    let mut violations = Vec::new();

    for file in files {
        let source = fs::read_to_string(file).unwrap_or_default();
        for needle in forbidden {
            if source.contains(needle) {
                violations.push(format!("{} contains {needle:?}", file.display()));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "architecture boundary violations:\n{}",
        violations.join("\n")
    );
}

#[test]
fn actions_do_not_depend_on_cli_or_command_parsing() {
    let root = crate_src_root().join("actions");
    let files = rust_files_under(&root);

    assert_sources_do_not_contain(
        &files,
        &["crate::cli", "crate::commands", "AppCommand", "clap::"],
    );
}

#[test]
fn daemon_internals_do_not_depend_on_cli_or_command_parsing() {
    let root = crate_src_root().join("daemon");
    let files = rust_files_under(&root);

    assert_sources_do_not_contain(
        &files,
        &["crate::cli", "crate::commands", "AppCommand", "clap::"],
    );
}

#[test]
fn event_decode_module_does_not_depend_on_recording() {
    let files = vec![crate_src_root().join("events/decode.rs")];

    assert_sources_do_not_contain(&files, &["crate::recorder", "recorder::", "LiveRecorder"]);
}

#[test]
fn policy_module_does_not_mutate_persistent_daemon_state() {
    let files = vec![crate_src_root().join("daemon/policy.rs")];

    assert_sources_do_not_contain(
        &files,
        &[
            "DaemonStateStore",
            "DaemonStateSnapshotWriter",
            "load_daemon_state",
            "default_daemon_state_snapshot_path",
        ],
    );
}
