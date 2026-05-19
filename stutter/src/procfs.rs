//! Typed procfs boundary helpers.

use std::path::Path;

pub use crate::error::ProcfsError;

pub(crate) fn read_procfs_to_string(path: &Path) -> Result<String, ProcfsError> {
    std::fs::read_to_string(path).map_err(|source| ProcfsError::Read {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_procfs_to_string_reports_typed_path_errors() {
        let err = read_procfs_to_string(Path::new("/definitely/not/a/procfs/file"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("failed to read procfs path"));
    }
}
