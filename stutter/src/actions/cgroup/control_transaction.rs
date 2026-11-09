//! Prepare, commit and restore for one typed cgroup control (package W11a).
//!
//! Every knob (`cpu.weight`, `cpu.idle`, `cpu.max`, `cpuset.*`, `memory.*`, `io.weight`) goes
//! through the same discipline, distinct from cgroup *placement*: preflight the value and the
//! hierarchy, capture the baseline text and the group's identity, check nothing moved between
//! prepare and commit, write, read back, and restore only what is still ours. A group that was
//! replaced or whose file changed under another owner is never overwritten.
use std::{
    fs, io,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

use super::control::{CgroupContext, CgroupControl, ControlDenial};

/// The file operations a control transaction performs; production uses the real files and tests
/// substitute a kernel stand-in (a clamping or racing file).
pub trait ControlFiles {
    fn read(&self, path: &Path) -> io::Result<String>;
    fn write(&self, path: &Path, text: &str) -> io::Result<()>;
}

pub struct RealFiles;

impl ControlFiles for RealFiles {
    fn read(&self, path: &Path) -> io::Result<String> {
        fs::read_to_string(path)
    }

    fn write(&self, path: &Path, text: &str) -> io::Result<()> {
        fs::write(path, text)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum PrepareError {
    Denied(ControlDenial),
    /// The control file is missing or unreadable: not applicable here.
    Unavailable,
    /// The group already has this value; nothing to experiment with.
    NoChange,
    /// The baseline cannot be written back exactly (per-device `io.weight` overrides).
    UnsupportedRestore,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CommitError {
    /// The group was replaced since prepare.
    GroupChanged,
    /// The file no longer holds the prepared baseline: another owner changed it.
    OwnerChanged,
    Write,
    /// The kernel did not apply the value; the baseline was written back (`restored`).
    ReadbackMismatch {
        restored: bool,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub enum RestoreOutcome {
    /// The baseline was written back and read back.
    Restored,
    /// The file already holds the baseline.
    AlreadyRestored,
    /// The file holds neither our value nor the baseline: not ours, left untouched.
    ExternallyChanged,
    /// The group or file is gone or was replaced.
    UnsafeToRestore,
    /// The baseline write did not read back.
    Unproven,
}

#[derive(Debug)]
pub struct ControlPrepared {
    dir: PathBuf,
    control: CgroupControl,
    baseline: String,
    identity: (u64, u64),
}

#[derive(Debug)]
pub struct ControlCommitted {
    dir: PathBuf,
    control: CgroupControl,
    baseline: String,
    identity: (u64, u64),
}

fn identity(dir: &Path) -> Option<(u64, u64)> {
    fs::metadata(dir).ok().map(|meta| (meta.dev(), meta.ino()))
}

/// The text that puts a baseline back. `io.weight` reads as `default N` plus overrides; only the
/// override-free form can be restored exactly.
fn restore_text(control: &CgroupControl, baseline: &str) -> Option<String> {
    match control {
        CgroupControl::IoWeight(_) => {
            let mut lines = baseline.lines().map(str::trim).filter(|l| !l.is_empty());
            let weight = lines.next()?.strip_prefix("default ")?.trim().to_owned();
            lines.next().is_none().then_some(weight)
        }
        _ => Some(baseline.trim().to_owned()),
    }
}

fn same(control: &CgroupControl, current: &str, baseline: &str) -> bool {
    match control {
        CgroupControl::IoWeight(_) => {
            restore_text(control, current) == restore_text(control, baseline)
        }
        _ => current.trim() == baseline.trim(),
    }
}

/// Validates the value and hierarchy (`io_weight_honored` is the separately verified device
/// mechanism, needed only for `io.weight`) and captures the baseline; writes nothing.
pub fn prepare(
    dir: &Path,
    control: &CgroupControl,
    euid: u32,
    io_weight_honored: Option<bool>,
    files: &dyn ControlFiles,
) -> Result<ControlPrepared, PrepareError> {
    let mut context = CgroupContext::read(dir, euid);
    // Presence of `io.weight` is not proof a device honors it: the caller verifies the mechanism.
    context.io_weight_honored = io_weight_honored;
    context.preflight(control).map_err(PrepareError::Denied)?;
    let (file, _) = control.file_and_controller();
    let baseline = files
        .read(&dir.join(file))
        .map_err(|_| PrepareError::Unavailable)?;
    let identity = identity(dir).ok_or(PrepareError::Unavailable)?;
    if control.readback_matches(&baseline) {
        return Err(PrepareError::NoChange);
    }
    if restore_text(control, &baseline).is_none() {
        return Err(PrepareError::UnsupportedRestore);
    }
    Ok(ControlPrepared {
        dir: dir.to_owned(),
        control: control.clone(),
        baseline,
        identity,
    })
}

impl ControlPrepared {
    pub fn baseline(&self) -> &str {
        self.baseline.trim()
    }

    pub fn commit(self, files: &dyn ControlFiles) -> Result<ControlCommitted, CommitError> {
        let (file, _) = self.control.file_and_controller();
        let path = self.dir.join(file);
        if identity(&self.dir) != Some(self.identity) {
            return Err(CommitError::GroupChanged);
        }
        match files.read(&path) {
            Ok(current) if same(&self.control, &current, &self.baseline) => {}
            _ => return Err(CommitError::OwnerChanged),
        }
        files
            .write(&path, &self.control.render())
            .map_err(|_| CommitError::Write)?;
        let applied = files
            .read(&path)
            .is_ok_and(|text| self.control.readback_matches(&text));
        if !applied {
            let restored = restore_text(&self.control, &self.baseline)
                .is_some_and(|text| files.write(&path, &text).is_ok());
            return Err(CommitError::ReadbackMismatch { restored });
        }
        Ok(ControlCommitted {
            dir: self.dir,
            control: self.control,
            baseline: self.baseline,
            identity: self.identity,
        })
    }
}

impl ControlCommitted {
    /// Writes the baseline back only while the file still holds our value.
    pub fn restore(self, files: &dyn ControlFiles) -> RestoreOutcome {
        let (file, _) = self.control.file_and_controller();
        let path = self.dir.join(file);
        if identity(&self.dir) != Some(self.identity) {
            return RestoreOutcome::UnsafeToRestore;
        }
        let Ok(current) = files.read(&path) else {
            return RestoreOutcome::UnsafeToRestore;
        };
        if same(&self.control, &current, &self.baseline) {
            return RestoreOutcome::AlreadyRestored;
        }
        if !self.control.readback_matches(&current) {
            return RestoreOutcome::ExternallyChanged;
        }
        let Some(text) = restore_text(&self.control, &self.baseline) else {
            return RestoreOutcome::UnsafeToRestore;
        };
        if files.write(&path, &text).is_err() {
            return RestoreOutcome::Unproven;
        }
        match files.read(&path) {
            Ok(after) if same(&self.control, &after, &self.baseline) => RestoreOutcome::Restored,
            _ => RestoreOutcome::Unproven,
        }
    }
}

#[cfg(test)]
#[path = "control_transaction_tests.rs"]
mod tests;
