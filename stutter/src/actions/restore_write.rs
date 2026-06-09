use std::{io, path::Path};

#[derive(Debug)]
pub(crate) enum RestoreWriteError {
    MissingTask,
    PermissionDenied(anyhow::Error),
    InvalidValue(anyhow::Error),
    Io(anyhow::Error),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct RestoreSummary {
    pub restored: usize,
    pub skipped_missing: usize,
    pub skipped_identity_mismatch: usize,
    pub failed: usize,
}

impl RestoreSummary {
    pub fn record_restored(&mut self) {
        self.restored += 1;
    }

    pub fn record_missing(&mut self) {
        self.skipped_missing += 1;
    }

    pub fn record_identity_mismatch(&mut self) {
        self.skipped_identity_mismatch += 1;
    }

    pub fn record_failure(&mut self) {
        self.failed += 1;
    }

    pub fn has_failures(self) -> bool {
        self.failed > 0
    }
}

pub(crate) fn classify_restore_write_error(
    proc_root: &Path,
    tid: u32,
    err: anyhow::Error,
) -> RestoreWriteError {
    if let Some(io_err) = err.downcast_ref::<io::Error>() {
        if io_err.raw_os_error() == Some(libc::ESRCH)
            || (io_err.kind() == io::ErrorKind::NotFound && !proc_task_exists(proc_root, tid))
        {
            return RestoreWriteError::MissingTask;
        }

        if io_err.kind() == io::ErrorKind::PermissionDenied {
            return RestoreWriteError::PermissionDenied(err);
        }

        if io_err.kind() == io::ErrorKind::InvalidInput
            || io_err.kind() == io::ErrorKind::InvalidData
            || io_err.raw_os_error() == Some(libc::EINVAL)
        {
            return RestoreWriteError::InvalidValue(err);
        }
    }

    RestoreWriteError::Io(err)
}

fn proc_task_exists(proc_root: &Path, tid: u32) -> bool {
    proc_root.join(tid.to_string()).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_esrch_as_missing_task() {
        let err = anyhow::Error::new(io::Error::from_raw_os_error(libc::ESRCH));

        assert!(matches!(
            classify_restore_write_error(Path::new("/missing-proc-root"), 42, err),
            RestoreWriteError::MissingTask
        ));
    }

    #[test]
    fn classifies_missing_proc_task_not_found_as_missing_task() {
        let err = anyhow::Error::new(io::Error::new(io::ErrorKind::NotFound, "gone"));

        assert!(matches!(
            classify_restore_write_error(Path::new("/missing-proc-root"), 42, err),
            RestoreWriteError::MissingTask
        ));
    }

    #[test]
    fn preserves_not_found_as_io_when_proc_task_still_exists() {
        let proc_root =
            std::env::temp_dir().join(format!("stutter-restore-write-{}", std::process::id()));
        let task_dir = proc_root.join("42");
        std::fs::create_dir_all(&task_dir).unwrap();
        let err = anyhow::Error::new(io::Error::new(io::ErrorKind::NotFound, "missing cgroup"));

        assert!(matches!(
            classify_restore_write_error(&proc_root, 42, err),
            RestoreWriteError::Io(_)
        ));

        std::fs::remove_dir_all(proc_root).ok();
    }

    #[test]
    fn classifies_permission_denied_as_real_failure_category() {
        let err = anyhow::Error::new(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "not allowed",
        ));

        assert!(matches!(
            classify_restore_write_error(Path::new("/missing-proc-root"), 42, err),
            RestoreWriteError::PermissionDenied(_)
        ));
    }

    #[test]
    fn classifies_invalid_value_as_real_failure_category() {
        let err = anyhow::Error::new(io::Error::new(io::ErrorKind::InvalidInput, "bad value"));

        assert!(matches!(
            classify_restore_write_error(Path::new("/missing-proc-root"), 42, err),
            RestoreWriteError::InvalidValue(_)
        ));
    }

    #[test]
    fn restore_summary_tracks_best_effort_outcome_counts() {
        let mut summary = RestoreSummary::default();
        summary.record_restored();
        summary.record_missing();
        summary.record_identity_mismatch();
        summary.record_failure();

        assert_eq!(
            summary,
            RestoreSummary {
                restored: 1,
                skipped_missing: 1,
                skipped_identity_mismatch: 1,
                failed: 1,
            }
        );
        assert!(summary.has_failures());
    }
}
