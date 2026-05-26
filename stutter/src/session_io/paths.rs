use std::path::{Path, PathBuf};

use crate::artifacts::{ArtifactKind, artifact_path};

pub(super) fn run_dir_for(path: &Path) -> PathBuf {
    if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    }
}

pub(super) fn artifact_input_path(path: &Path, kind: ArtifactKind) -> PathBuf {
    if path.is_dir() {
        artifact_path(path, kind)
    } else {
        path.to_path_buf()
    }
}

pub(super) fn push_unique_string(values: &mut Vec<String>, value: impl Into<String>) {
    let value = value.into();
    if !values.contains(&value) {
        values.push(value);
    }
}

pub(super) fn file_name_for_path(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .unwrap_or_else(|| path.display().to_string())
}
