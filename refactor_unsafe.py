import os
import re

# We will create stutter/src/syscall.rs and stutter/src/ffi.rs
syscall_rs_content = """//! Safe wrappers around libc syscalls.

use std::{io, mem, path::Path};
use std::os::unix::ffi::OsStrExt;

pub fn clock_ticks_per_second() -> u64 {
    // SAFETY: _SC_CLK_TCK is a valid sysconf name and has no side effects.
    let value = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if value <= 0 {
        100 // fallback
    } else {
        value as u64
    }
}

pub struct DiskSpace {
    pub free_bytes: u64,
    pub total_bytes: u64,
}

pub fn statvfs(path: &Path) -> io::Result<DiskSpace> {
    let mut c_path = path.as_os_str().as_bytes().to_vec();
    c_path.push(0);
    let mut stat = mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: c_path is null-terminated, stat points to valid uninitialized memory.
    let rc = unsafe { libc::statvfs(c_path.as_ptr() as *const libc::c_char, stat.as_mut_ptr()) };
    if rc == 0 {
        // SAFETY: statvfs initialized the struct on success.
        let stat = unsafe { stat.assume_init() };
        Ok(DiskSpace {
            free_bytes: (stat.f_bfree as u64).saturating_mul(stat.f_frsize as u64),
            total_bytes: (stat.f_blocks as u64).saturating_mul(stat.f_frsize as u64),
        })
    } else {
        Err(io::Error::last_os_error())
    }
}

pub fn geteuid() -> u32 {
    // SAFETY: geteuid is always safe to call.
    unsafe { libc::geteuid() as u32 }
}

pub fn get_memlock_rlimit() -> io::Result<u64> {
    let mut limit = mem::MaybeUninit::<libc::rlimit>::uninit();
    // SAFETY: limit points to valid uninitialized memory.
    let rc = unsafe { libc::getrlimit(libc::RLIMIT_MEMLOCK, limit.as_mut_ptr()) };
    if rc == 0 {
        // SAFETY: getrlimit initialized the struct on success.
        let limit = unsafe { limit.assume_init() };
        Ok(limit.rlim_cur)
    } else {
        Err(io::Error::last_os_error())
    }
}

pub fn get_nofile_rlimit_max() -> io::Result<u64> {
    let mut limit = mem::MaybeUninit::<libc::rlimit>::uninit();
    // SAFETY: limit points to valid uninitialized memory.
    let rc = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, limit.as_mut_ptr()) };
    if rc == 0 {
        // SAFETY: getrlimit initialized the struct on success.
        let limit = unsafe { limit.assume_init() };
        Ok(limit.rlim_max)
    } else {
        Err(io::Error::last_os_error())
    }
}

pub fn gettid() -> u32 {
    // SAFETY: SYS_gettid is always safe to call.
    unsafe { libc::syscall(libc::SYS_gettid) as u32 }
}
"""

with open("stutter/src/syscall.rs", "w") as f:
    f.write(syscall_rs_content)

