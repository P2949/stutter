//! Community-rules process-name normalization.
//!
//! Owns conversion of process identity strings into stable rule lookup keys and guarded-name
//! policy. Does not own database indexing, classification, loading, or command handling.

use std::path::Path;

pub fn is_guarded_community_rule_name(normalized_name: &str) -> bool {
    const GUARDED_NAMES: &[&str] = &[
        "python",
        "python3",
        "java",
        "node",
        "wine",
        "bash",
        "sh",
        "zsh",
        "steam",
        "steamwebhelper",
        "electron",
        "chrome",
        "firefox",
        "setup.exe",
        "launcher.exe",
        "client.exe",
        "server.exe",
        "main.exe",
        "build.exe",
        "run.exe",
        "app.exe",
        "game.exe",
        "start.exe",
    ];

    if GUARDED_NAMES.contains(&normalized_name) {
        return true;
    }

    let stem = normalized_name
        .strip_suffix(".exe")
        .unwrap_or(normalized_name);

    stem.len() <= 3
        || matches!(
            stem,
            "app" | "run" | "main" | "start" | "setup" | "client" | "server"
        )
}

pub fn normalize_process_name(value: &str) -> Option<String> {
    let mut value = value.trim();
    while let Some(stripped) = value.strip_suffix(" (deleted)") {
        value = stripped.trim_end();
    }

    value = value.trim_matches('"').trim_matches('\'');
    if value.is_empty() {
        return None;
    }

    let slash_normalized = value.replace('\\', "/");
    let basename = Path::new(&slash_normalized)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(slash_normalized.as_str())
        .trim();

    if basename.is_empty() {
        None
    } else {
        Some(basename.to_ascii_lowercase())
    }
}
