use std::{
    fs,
    path::{Path, PathBuf},
};

fn repo_root() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    dir.pop();
    dir
}

fn markdown_files_under(path: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_markdown_files(path, &mut files);
    files.sort();
    files
}

fn collect_markdown_files(path: &Path, files: &mut Vec<PathBuf>) {
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };
    if metadata.is_file() {
        if path.extension().and_then(|ext| ext.to_str()) == Some("md") {
            files.push(path.to_path_buf());
        }
        return;
    }

    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let entry_path = entry.path();
        if entry_path.is_dir() {
            let name = entry_path.file_name().unwrap_or_default().to_string_lossy();
            if name == "target" || name == ".git" {
                continue;
            }
        }
        collect_markdown_files(&entry_path, files);
    }
}

#[test]
fn artifact_schema_doc_mentions_current_session_schema_version() {
    let doc = fs::read_to_string(repo_root().join("docs/ARTIFACT_SCHEMA.md"))
        .expect("docs/ARTIFACT_SCHEMA.md should be readable");
    let expected = format!(
        "that resolves to `{}`",
        crate::recorder::SESSION_SCHEMA_VERSION.get()
    );

    assert!(
        doc.contains(&expected),
        "docs/ARTIFACT_SCHEMA.md must mention the current SESSION_SCHEMA_VERSION"
    );
}

#[test]
fn documentation_rejects_stale_ebpf_build_command() {
    let mut bad_matches = Vec::new();

    for file in markdown_files_under(&repo_root()) {
        let content = fs::read_to_string(&file).unwrap_or_default();
        for (i, line) in content.lines().enumerate() {
            if line.contains("cargo build -p stutter-ebpf --release")
                && !line.contains("wrong command")
            {
                bad_matches.push(format!("{}:{}", file.display(), i + 1));
            }
        }
    }

    assert!(
        bad_matches.is_empty(),
        "Documentation must not recommend the stale eBPF release build. Use xtask or explicit bpf target instead.\nFound in:\n{}",
        bad_matches.join("\n")
    );
}
