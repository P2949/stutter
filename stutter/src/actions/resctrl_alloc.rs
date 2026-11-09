//! resctrl L3 cache allocation as a prepared transaction (package W11c).
//!
//! Separate from the read-only monitor. A Stutter-owned control group is created with a bounded,
//! contiguous capacity bitmask; the root (default) group's `schemata` is the bystander guard and
//! must be unchanged after every step. Restore removes only the group this transaction created,
//! and only while the group's identity is unchanged; the kernel returns any tasks to the default
//! group. Task assignment is a separate typed step and is not performed here. Experimental,
//! never autonomous, and absent hardware yields `Unavailable`, not a pass.
use std::{
    fs, io,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

const GROUP_PREFIX: &str = "stutter-cat-";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatRequest {
    /// Group name suffix, unique per transaction.
    pub name: String,
    /// Contiguous capacity bitmask to give the group on every L3 domain.
    pub mask: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CatDenial {
    /// resctrl is not mounted or has no L3 allocation.
    Unavailable,
    BadName,
    /// Zero, non-contiguous, below the hardware minimum width or outside the supported mask.
    BadMask,
    /// No free class of service remains.
    NoFreeClosid,
    GroupExists,
    /// The root schemata could not be read or has no L3 line.
    RootUnreadable,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CatCommitError {
    CreateFailed,
    /// The group was created but its schemata did not read back as requested; it was removed.
    ReadbackMismatch {
        removed: bool,
    },
    /// The default group's allocation changed during the commit: not ours; group removed.
    BystanderChanged {
        removed: bool,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub enum CatRestore {
    Removed,
    /// Already gone.
    Absent,
    /// The path now names a different group.
    UnsafeToRestore,
    /// Removal failed.
    Unproven,
}

#[derive(Debug)]
pub struct CatPrepared {
    group: PathBuf,
    schemata: String,
    root_schemata: String,
}

#[derive(Debug)]
pub struct CatCommitted {
    group: PathBuf,
    identity: (u64, u64),
}

fn read(path: &Path) -> io::Result<String> {
    fs::read_to_string(path)
}

fn contiguous(mask: u64) -> bool {
    mask != 0 && {
        let shifted = mask >> mask.trailing_zeros();
        shifted & (shifted + 1) == 0
    }
}

fn value(path: &Path) -> Option<u64> {
    read(path).ok()?.trim().parse().ok()
}

fn hex(path: &Path) -> Option<u64> {
    u64::from_str_radix(read(path).ok()?.trim(), 16).ok()
}

/// The domain ids in the root's `L3:0=fff;1=fff` line.
fn l3_domains(schemata: &str) -> Option<Vec<String>> {
    let line = schemata
        .lines()
        .find_map(|l| l.trim().strip_prefix("L3:"))?;
    let ids: Vec<String> = line
        .split(';')
        .filter_map(|part| part.split_once('=').map(|(id, _)| id.trim().to_owned()))
        .collect();
    (!ids.is_empty()).then_some(ids)
}

/// Validates against the mounted hierarchy and captures the bystander snapshot; writes nothing.
pub fn prepare(root: &Path, request: &CatRequest) -> Result<CatPrepared, CatDenial> {
    let info = root.join("info/L3");
    let cbm_mask = hex(&info.join("cbm_mask")).ok_or(CatDenial::Unavailable)?;
    let min_bits = value(&info.join("min_cbm_bits")).unwrap_or(1);
    let closids = value(&info.join("num_closids")).ok_or(CatDenial::Unavailable)?;
    if request.name.is_empty()
        || request.name.len() > 32
        || !request
            .name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(CatDenial::BadName);
    }
    if !contiguous(request.mask)
        || request.mask & !cbm_mask != 0
        || u64::from(request.mask.count_ones()) < min_bits
    {
        return Err(CatDenial::BadMask);
    }
    let root_schemata = read(&root.join("schemata")).map_err(|_| CatDenial::RootUnreadable)?;
    let domains = l3_domains(&root_schemata).ok_or(CatDenial::RootUnreadable)?;
    // The default group uses one class of service, every group directory one more, and the new
    // group needs one of its own.
    let groups = fs::read_dir(root)
        .map_err(|_| CatDenial::Unavailable)?
        .flatten()
        .filter(|entry| entry.path().join("schemata").is_file())
        .count() as u64;
    if groups + 2 > closids {
        return Err(CatDenial::NoFreeClosid);
    }
    let group = root.join(format!("{GROUP_PREFIX}{}", request.name));
    if group.exists() {
        return Err(CatDenial::GroupExists);
    }
    let masks = domains
        .iter()
        .map(|id| format!("{id}={:x}", request.mask))
        .collect::<Vec<_>>()
        .join(";");
    Ok(CatPrepared {
        group,
        schemata: format!("L3:{masks}"),
        root_schemata,
    })
}

impl CatPrepared {
    pub fn schemata(&self) -> &str {
        &self.schemata
    }

    pub fn commit(self) -> Result<CatCommitted, CatCommitError> {
        fs::create_dir(&self.group).map_err(|_| CatCommitError::CreateFailed)?;
        let identity = fs::metadata(&self.group)
            .map(|meta| (meta.dev(), meta.ino()))
            .map_err(|_| CatCommitError::CreateFailed)?;
        let remove = || remove_group(&self.group).is_ok();
        let wrote = fs::write(self.group.join("schemata"), format!("{}\n", self.schemata)).is_ok();
        let applied = wrote
            && read(&self.group.join("schemata"))
                .ok()
                .is_some_and(|text| text.lines().any(|line| line.trim() == self.schemata));
        if !applied {
            return Err(CatCommitError::ReadbackMismatch { removed: remove() });
        }
        let root = self.group.parent().unwrap_or(&self.group);
        if read(&root.join("schemata")).ok().as_deref() != Some(self.root_schemata.as_str()) {
            return Err(CatCommitError::BystanderChanged { removed: remove() });
        }
        Ok(CatCommitted {
            group: self.group,
            identity,
        })
    }
}

impl CatCommitted {
    /// Removes the group this transaction created, if the path still names it.
    pub fn restore(self) -> CatRestore {
        match fs::metadata(&self.group) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => CatRestore::Absent,
            Err(_) => CatRestore::Unproven,
            Ok(meta) if (meta.dev(), meta.ino()) != self.identity => CatRestore::UnsafeToRestore,
            Ok(_) => {
                if remove_group(&self.group).is_ok() {
                    CatRestore::Removed
                } else {
                    CatRestore::Unproven
                }
            }
        }
    }
}

/// resctrl is a kernfs tree: `rmdir` removes a group with its control files. A fake root is a plain
/// directory, so tests clear its files first.
fn remove_group(path: &Path) -> io::Result<()> {
    #[cfg(test)]
    for entry in fs::read_dir(path)?.flatten() {
        fs::remove_file(entry.path())?;
    }
    fs::remove_dir(path)
}

#[cfg(test)]
#[path = "resctrl_alloc_tests.rs"]
mod tests;
