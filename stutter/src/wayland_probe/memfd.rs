use std::{fs::File, os::fd::FromRawFd};

use anyhow::{Context, Result};

pub(crate) fn create_memfd(size: usize) -> Result<File> {
    let name = c"stutter-wayland-probe";
    // SAFETY: name is a static NUL-terminated C string and flags are valid
    // for memfd_create; errors are handled below.
    let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error()).context("memfd_create failed");
    }
    // SAFETY: fd was just returned by memfd_create and ownership is being
    // transferred exactly once into File.
    let file = unsafe { File::from_raw_fd(fd) };
    file.set_len(size as u64)?;
    Ok(file)
}
