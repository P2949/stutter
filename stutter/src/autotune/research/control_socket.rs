//! A read-only inspection of the resident controller's control socket (PLAN G1: a resident service
//! with a safe IPC and a single owner). The socket is the only way to submit, pause, cancel or read a
//! campaign, and the kernel decides who may connect: a socket in a private directory with mode 0600
//! owned by the campaign's user can be reached only by that user, and the service then checks the
//! connecting process's credentials again. This stats the directory and the socket and reads the lock
//! table; it connects to nothing and changes nothing. It does not replace the credential check, which is
//! tested against real socket pairs in `protocol`.

use std::path::Path;

/// The socket's file name inside the private directory.
pub const SOCKET_NAME: &str = "research.sock";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlSocketFacts {
    /// The directory is a real directory owned by the user with no group or other access.
    pub directory_private: bool,
    /// The socket is a socket (not a link or file), owned by the user, with no group or other access
    /// and a single link.
    pub socket_private: bool,
    /// A live process holds the directory's exclusive lock, which is how the resident controller
    /// claims it (`None`: the lock table could not be read).
    pub controller_holds_directory: Option<bool>,
}

/// Inspect the control socket under `dir` for `uid`; `None` when the directory cannot be statted.
pub fn inspect(dir: &Path, uid: u32) -> Option<ControlSocketFacts> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    let meta = std::fs::symlink_metadata(dir).ok()?;
    let socket = std::fs::symlink_metadata(dir.join(SOCKET_NAME)).ok();
    Some(ControlSocketFacts {
        directory_private: meta.is_dir() && meta.uid() == uid && meta.mode() & 0o077 == 0,
        socket_private: socket.is_some_and(|socket| {
            socket.file_type().is_socket()
                && socket.uid() == uid
                && socket.nlink() == 1
                && socket.mode() & 0o077 == 0
        }),
        controller_holds_directory: super::lease_inspect::flock_held(&meta),
    })
}

/// G1's blockers from an inspection (`None`: the socket directory was not inspected).
pub fn blockers(facts: Option<ControlSocketFacts>) -> Vec<&'static str> {
    let Some(facts) = facts else {
        return vec!["ControlSocketNotInspected"];
    };
    let mut blockers = Vec::new();
    if !facts.directory_private {
        blockers.push("ControlDirectoryNotPrivate");
    }
    if !facts.socket_private {
        blockers.push("ControlSocketNotPrivate");
    }
    match facts.controller_holds_directory {
        Some(true) => {}
        Some(false) => blockers.push("NoControllerHoldsTheControlDirectory"),
        None => blockers.push("ControlDirectoryOwnerUnknown"),
    }
    blockers
}

#[cfg(test)]
#[path = "control_socket_tests.rs"]
mod tests;
