//! Safe, opened-descriptor confinement of capture directories and files.
//!
//! The directory is opened with `O_NOFOLLOW | O_DIRECTORY` and checked on the descriptor, and a
//! file is opened relative to that descriptor, so a symlink, a traversal, a special file or a
//! swapped inode is refused instead of followed.
use std::{
    fs::File,
    io,
    os::{
        fd::{AsFd, OwnedFd},
        unix::fs::MetadataExt,
    },
    path::Path,
};

/// Device, inode and owner of an opened file; a later change means a different source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileIdentity {
    pub dev: u64,
    pub ino: u64,
    pub uid: u32,
}

#[derive(Debug)]
pub enum ConfinementError {
    NotADirectory,
    WrongOwner,
    PermissiveMode,
    BadName,
    NotRegularFile,
    Io(io::Error),
}

impl From<io::Error> for ConfinementError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// A capture directory verified on its open descriptor.
pub struct ConfinedDir {
    fd: OwnedFd,
    owner_uid: u32,
}

impl ConfinedDir {
    /// Opens `path` as a directory owned by `owner_uid` with no group/other access.
    pub fn open(path: &Path, owner_uid: u32) -> Result<Self, ConfinementError> {
        let fd = crate::syscall::open_dir_nofollow(path).map_err(|error| {
            match error.raw_os_error() {
                Some(libc::ELOOP | libc::ENOTDIR) => ConfinementError::NotADirectory,
                _ => ConfinementError::Io(error),
            }
        })?;
        let meta = File::from(fd.try_clone()?).metadata()?;
        if !meta.is_dir() {
            return Err(ConfinementError::NotADirectory);
        }
        if meta.uid() != owner_uid {
            return Err(ConfinementError::WrongOwner);
        }
        if meta.mode() & 0o077 != 0 {
            return Err(ConfinementError::PermissiveMode);
        }
        Ok(Self { fd, owner_uid })
    }

    /// Opens regular file `name` inside this directory, refusing any path component, symlink or
    /// special file, and returns it with its identity.
    pub fn open_file(&self, name: &str) -> Result<(File, FileIdentity), ConfinementError> {
        if name.is_empty()
            || name == "."
            || name == ".."
            || name.contains('/')
            || name.contains('\0')
        {
            return Err(ConfinementError::BadName);
        }
        let file = crate::syscall::openat_nofollow(self.fd.as_fd(), name)?;
        let meta = file.metadata()?;
        if !meta.is_file() {
            return Err(ConfinementError::NotRegularFile);
        }
        if meta.uid() != self.owner_uid {
            return Err(ConfinementError::WrongOwner);
        }
        let identity = FileIdentity {
            dev: meta.dev(),
            ino: meta.ino(),
            uid: meta.uid(),
        };
        Ok((file, identity))
    }
}
