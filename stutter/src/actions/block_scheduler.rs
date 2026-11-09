//! Device-wide block I/O scheduler selection as a prepared transaction (package W11e).
//!
//! `queue/scheduler` reads like `none [mq-deadline] kyber bfq`: the bracketed name is active and
//! the rest are the ones this device and kernel really support. A change is device-wide, so it is
//! only prepared for a device that was explicitly authorized as disposable, never for a protected
//! (root or boot) device, and only to a supported scheduler. The device is recorded by its
//! major:minor number so restore never writes to a different device that reused the name.
use std::{
    collections::BTreeSet,
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct BlockSchedulerRequest {
    /// Kernel device name, such as `nvme1n1`; a single path component.
    pub device: String,
    pub scheduler: String,
    /// Devices explicitly authorized for device-wide scheduler experiments.
    pub authorized: BTreeSet<String>,
    /// Root, boot and other devices that must never be changed.
    pub protected: BTreeSet<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum BlockSchedulerDenial {
    BadName,
    /// No `queue/scheduler` file: not a scheduler-capable block device here.
    Unavailable,
    NotAuthorized,
    Protected,
    /// The kernel does not list this scheduler for this device.
    UnsupportedScheduler,
    /// Already active: nothing to experiment with.
    NoChange,
    /// The device number could not be read, so the device cannot be identified for restore.
    NoDeviceNumber,
}

#[derive(Debug, PartialEq, Eq)]
pub enum BlockSchedulerCommitError {
    Write,
    /// The kernel did not report the requested scheduler active; the original was written back.
    ReadbackMismatch {
        restored: bool,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub enum BlockSchedulerRestore {
    Restored,
    AlreadyRestored,
    /// Neither our scheduler nor the original is active: another owner changed it.
    ExternallyChanged,
    /// The name now refers to a different device (or none).
    UnsafeToRestore,
    Unproven,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BlockSchedulerRecord {
    pub queue: PathBuf,
    pub device_number: String,
    pub original: String,
    pub applied: String,
}

#[derive(Debug)]
pub struct BlockSchedulerPrepared {
    record: BlockSchedulerRecord,
}

#[derive(Debug)]
pub struct BlockSchedulerCommitted {
    record: BlockSchedulerRecord,
}

fn parse(text: &str) -> Option<(Vec<String>, String)> {
    let mut active = None;
    let mut all = Vec::new();
    for word in text.split_whitespace() {
        if let Some(name) = word.strip_prefix('[').and_then(|w| w.strip_suffix(']')) {
            active = Some(name.to_owned());
            all.push(name.to_owned());
        } else {
            all.push(word.to_owned());
        }
    }
    Some((all, active?))
}

/// The file operations on `queue/scheduler`; tests substitute the kernel's bracket behaviour.
pub trait BlockFiles {
    fn read(&self, path: &Path) -> io::Result<String>;
    fn write(&self, path: &Path, text: &str) -> io::Result<()>;
}

pub struct RealBlockFiles;

impl BlockFiles for RealBlockFiles {
    fn read(&self, path: &Path) -> io::Result<String> {
        fs::read_to_string(path)
    }

    fn write(&self, path: &Path, text: &str) -> io::Result<()> {
        fs::write(path, text)
    }
}

fn active(files: &dyn BlockFiles, queue: &Path) -> Option<String> {
    parse(&files.read(queue).ok()?).map(|(_, active)| active)
}

fn device_number(sys_block: &Path, device: &str) -> Option<String> {
    Some(
        fs::read_to_string(sys_block.join(device).join("dev"))
            .ok()?
            .trim()
            .to_owned(),
    )
    .filter(|n| !n.is_empty())
}

/// Validates the request against the device's own scheduler list; writes nothing.
pub fn prepare(
    sys_block: &Path,
    request: &BlockSchedulerRequest,
) -> Result<BlockSchedulerPrepared, BlockSchedulerDenial> {
    if request.device.is_empty()
        || request.device.contains('/')
        || request.device.starts_with('.')
        || request.scheduler.is_empty()
        || !request
            .scheduler
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(BlockSchedulerDenial::BadName);
    }
    let queue = sys_block.join(&request.device).join("queue/scheduler");
    let text = fs::read_to_string(&queue).map_err(|_| BlockSchedulerDenial::Unavailable)?;
    let (supported, original) = parse(&text).ok_or(BlockSchedulerDenial::Unavailable)?;
    if request.protected.contains(&request.device) {
        return Err(BlockSchedulerDenial::Protected);
    }
    if !request.authorized.contains(&request.device) {
        return Err(BlockSchedulerDenial::NotAuthorized);
    }
    if !supported.contains(&request.scheduler) {
        return Err(BlockSchedulerDenial::UnsupportedScheduler);
    }
    if original == request.scheduler {
        return Err(BlockSchedulerDenial::NoChange);
    }
    let device_number =
        device_number(sys_block, &request.device).ok_or(BlockSchedulerDenial::NoDeviceNumber)?;
    Ok(BlockSchedulerPrepared {
        record: BlockSchedulerRecord {
            queue,
            device_number,
            original,
            applied: request.scheduler.clone(),
        },
    })
}

fn put(files: &dyn BlockFiles, queue: &Path, name: &str) -> io::Result<()> {
    files.write(queue, name)
}

impl BlockSchedulerPrepared {
    pub fn commit(
        self,
        files: &dyn BlockFiles,
    ) -> Result<BlockSchedulerCommitted, BlockSchedulerCommitError> {
        let record = self.record;
        put(files, &record.queue, &record.applied).map_err(|_| BlockSchedulerCommitError::Write)?;
        if active(files, &record.queue).as_deref() != Some(record.applied.as_str()) {
            let restored = put(files, &record.queue, &record.original).is_ok();
            return Err(BlockSchedulerCommitError::ReadbackMismatch { restored });
        }
        Ok(BlockSchedulerCommitted { record })
    }
}

impl BlockSchedulerCommitted {
    pub fn record(&self) -> BlockSchedulerRecord {
        self.record.clone()
    }
}

/// Restores the original scheduler only on the same device (by number) and only while ours is
/// still active.
pub fn restore_recorded(
    sys_block: &Path,
    record: &BlockSchedulerRecord,
    files: &dyn BlockFiles,
) -> BlockSchedulerRestore {
    let device = record
        .queue
        .parent()
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .and_then(|name| name.to_str());
    let Some(device) = device else {
        return BlockSchedulerRestore::UnsafeToRestore;
    };
    if device_number(sys_block, device).as_deref() != Some(record.device_number.as_str()) {
        return BlockSchedulerRestore::UnsafeToRestore;
    }
    match active(files, &record.queue) {
        Some(now) if now == record.original => BlockSchedulerRestore::AlreadyRestored,
        Some(now) if now == record.applied => {
            if put(files, &record.queue, &record.original).is_err() {
                return BlockSchedulerRestore::Unproven;
            }
            if active(files, &record.queue).as_deref() == Some(record.original.as_str()) {
                BlockSchedulerRestore::Restored
            } else {
                BlockSchedulerRestore::Unproven
            }
        }
        Some(_) => BlockSchedulerRestore::ExternallyChanged,
        None => BlockSchedulerRestore::UnsafeToRestore,
    }
}

#[cfg(test)]
#[path = "block_scheduler_tests.rs"]
mod tests;
