//! Private birth records: cgroup v2 forbids rename, so record its kernel identity before exec.
use std::os::unix::{ffi::OsStrExt, fs::DirBuilderExt};

use serde::{Deserialize, Serialize};

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct KernelId {
    pub device: u64,
    pub inode: u64,
}
impl KernelId {
    pub fn of(file: &File) -> io::Result<Self> {
        let metadata = file.metadata()?;
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Record {
    pub version: u8,
    pub boot: String,
    pub uid: u32,
    pub pid: u32,
    pub ticks: u64,
    pub parent: PathBuf,
    pub parent_id: KernelId,
    pub name: String,
    pub resource: Option<KernelId>,
}

pub(super) struct Registry {
    pub path: PathBuf,
    pub uid: u32,
    pub boot: String,
}
pub(super) struct Birth {
    pub registry: Registry,
    pub record: Record,
}

fn private_file(path: &Path, uid: u32) -> anyhow::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    if metadata.nlink() == 0 {
        return Err(io::Error::new(io::ErrorKind::NotFound, "birth record was retired").into());
    }
    ensure!(
        metadata.is_file()
            && metadata.uid() == uid
            && metadata.mode() & 0o077 == 0
            && metadata.nlink() == 1,
        "invalid validation birth record ownership or file type"
    );
    Ok(file)
}

impl Registry {
    pub fn open() -> anyhow::Result<Self> {
        // SAFETY: geteuid has no preconditions.
        let uid = unsafe { libc::geteuid() };
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        ensure!(
            base.is_absolute(),
            "validation registry requires an absolute runtime directory"
        );
        let base = base.canonicalize()?;
        // Root never reads a user-owned registry. For an unprivileged caller, entries protected
        // by root or that same caller are acceptable; other owners cannot replace the path.
        for component in base.ancestors() {
            let metadata = fs::metadata(component)?;
            ensure!(
                (metadata.uid() == 0 || metadata.uid() == uid)
                    && (metadata.mode() & 0o022 == 0 || metadata.mode() & 0o1000 != 0),
                "unprotected validation registry ancestor"
            );
        }
        let path = base.join(format!("stutter-validation-containment-{uid}"));
        match fs::DirBuilder::new().mode(0o700).create(&path) {
            Ok(()) => (),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
            Err(error) => return Err(error.into()),
        }
        let metadata = fs::symlink_metadata(&path)?;
        ensure!(
            metadata.is_dir() && metadata.uid() == uid && metadata.mode() & 0o077 == 0,
            "validation registry must be a private owned directory"
        );
        let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id")?
            .trim()
            .to_owned();
        ensure!(
            !boot.is_empty() && boot.len() <= 64,
            "invalid kernel boot identity"
        );
        Ok(Self { path, uid, boot })
    }

    pub fn intent(
        self,
        parent: &Path,
        name: String,
        pid: u32,
        ticks: u64,
    ) -> anyhow::Result<Birth> {
        let parent_file = cgroup_file(parent, false, true)?;
        let record = Record {
            version: 1,
            boot: self.boot.clone(),
            uid: self.uid,
            pid,
            ticks,
            parent: parent.to_owned(),
            parent_id: KernelId::of(&parent_file)?,
            name,
            resource: None,
        };
        let birth = Birth {
            registry: self,
            record,
        };
        birth.write(true)?;
        Ok(birth)
    }

    pub fn records(&self) -> anyhow::Result<Vec<Record>> {
        let mut records = Vec::new();
        for (index, entry) in fs::read_dir(&self.path)?.enumerate() {
            ensure!(
                index < 4096,
                "validation birth registry entry bound exceeded"
            );
            let entry = entry?;
            if let Some(name) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.strip_suffix(".json.tmp"))
            {
                if let Some((pid, ticks)) = cleanup::parse_owner(name)
                    && cleanup::owner_gone(pid, ticks)
                {
                    // An unpublished initial intent cannot have launched a workload. Retire
                    // only this private temporary file; never infer a cgroup from its body.
                    match private_file(&entry.path(), self.uid) {
                        Ok(_) => match fs::remove_file(entry.path()) {
                            Ok(()) => (),
                            Err(error) if error.kind() == io::ErrorKind::NotFound => (),
                            Err(error) => return Err(error.into()),
                        },
                        Err(error)
                            if error
                                .downcast_ref::<io::Error>()
                                .is_some_and(|e| e.kind() == io::ErrorKind::NotFound) => {}
                        Err(error) => return Err(error),
                    }
                }
                continue;
            }
            if entry.path().extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            let mut data = Vec::new();
            let file = match private_file(&entry.path(), self.uid) {
                Ok(file) => file,
                Err(error)
                    if error
                        .downcast_ref::<io::Error>()
                        .is_some_and(|e| e.kind() == io::ErrorKind::NotFound) =>
                {
                    continue;
                }
                Err(error) => return Err(error),
            };
            file.take(8193).read_to_end(&mut data)?;
            ensure!(data.len() <= 8192, "validation birth record too large");
            let record: Record = serde_json::from_slice(&data)?;
            ensure!(
                record.version == 1
                    && record.uid == self.uid
                    && cleanup::parse_owner(&record.name) == Some((record.pid, record.ticks))
                    && entry.file_name() == format!("{}.json", record.name).as_str(),
                "invalid validation birth record identity"
            );
            records.push(record);
        }
        Ok(records)
    }

    pub fn remove(&self, record: &Record) -> anyhow::Result<()> {
        let path = self.path.join(format!("{}.json", record.name));
        match private_file(&path, self.uid) {
            Ok(_) => (),
            Err(error)
                if error
                    .downcast_ref::<io::Error>()
                    .is_some_and(|e| e.kind() == io::ErrorKind::NotFound) =>
            {
                return Ok(());
            }
            Err(error) => return Err(error),
        }
        match fs::remove_file(path) {
            Ok(()) => (),
            Err(error) if error.kind() == io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
        let _ = fs::remove_file(self.path.join(format!("{}.json.tmp", record.name)));
        File::open(&self.path)?.sync_all()?;
        Ok(())
    }
}

impl Birth {
    pub fn complete(&mut self, resource: &File) -> anyhow::Result<()> {
        let resource = validate_cgroup_file(resource.try_clone()?)?;
        ensure!(
            resource.metadata()?.is_dir(),
            "validation birth requires a kernel cgroup directory"
        );
        ensure!(
            resource.metadata()?.uid() == self.registry.uid,
            "validation cgroup changed owner before birth"
        );
        let previous = self.record.resource;
        self.record.resource = Some(KernelId::of(&resource)?);
        if let Err(error) = self.write(false) {
            self.record.resource = previous;
            return Err(error);
        }
        Ok(())
    }
    fn write(&self, initial: bool) -> anyhow::Result<()> {
        let path = self
            .registry
            .path
            .join(format!("{}.json", self.record.name));
        // Never expose partially written JSON to concurrent stage startup/reconciliation.
        let destination = path.with_extension("json.tmp");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&destination)?;
        let result = (|| -> anyhow::Result<()> {
            file.write_all(&serde_json::to_vec(&self.record)?)?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            if initial {
                let from = std::ffi::CString::new(destination.as_os_str().as_bytes())?;
                let to = std::ffi::CString::new(path.as_os_str().as_bytes())?;
                // SAFETY: both strings are live NUL-terminated paths; NOREPLACE preserves any
                // existing stage record rather than silently taking ownership of it.
                if unsafe {
                    libc::renameat2(
                        libc::AT_FDCWD,
                        from.as_ptr(),
                        libc::AT_FDCWD,
                        to.as_ptr(),
                        libc::RENAME_NOREPLACE,
                    )
                } != 0
                {
                    return Err(io::Error::last_os_error().into());
                }
            } else {
                fs::rename(&destination, &path)?;
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&destination);
        }
        result?;
        File::open(&self.registry.path)?.sync_all()?;
        Ok(())
    }
    pub fn remove(&self) -> anyhow::Result<()> {
        self.registry.remove(&self.record)
    }
}

#[cfg(test)]
#[path = "containment_journal_tests.rs"]
mod tests;
