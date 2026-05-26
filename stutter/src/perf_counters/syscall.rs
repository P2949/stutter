use std::{io, os::unix::io::RawFd};

use super::group::PerfEventAttr;

pub fn perf_event_open(
    attr: &mut PerfEventAttr,
    pid: libc::pid_t,
    cpu: libc::c_int,
    group_fd: RawFd,
    flags: libc::c_ulong,
) -> io::Result<RawFd> {
    // SAFETY: attr points to a valid perf_event_attr-compatible C struct for
    // the duration of the syscall; remaining arguments are plain integers.
    let fd = unsafe {
        libc::syscall(
            libc::SYS_perf_event_open,
            attr as *mut PerfEventAttr,
            pid,
            cpu,
            group_fd,
            flags,
        )
    };
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(fd as RawFd)
    }
}

pub fn perf_ioctl(fd: RawFd, request: libc::c_ulong, arg: libc::c_ulong) -> io::Result<()> {
    // SAFETY: ioctl is called on a perf fd owned by this module, with requests
    // that take an integer argument rather than a user pointer.
    let ret = unsafe { libc::ioctl(fd, request, arg) };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn read_counters(fd: RawFd, values: &mut [u64], expected_bytes: usize) -> io::Result<usize> {
    // SAFETY: values has capacity for the maximum configured group payload,
    // and expected_bytes is derived from the active event count.
    let read_bytes = unsafe {
        libc::read(
            fd,
            values.as_mut_ptr().cast::<libc::c_void>(),
            expected_bytes,
        )
    };
    if read_bytes < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(read_bytes as usize)
    }
}

pub fn close_fd(fd: RawFd) {
    // SAFETY: fd is owned by this wrapper and close is idempotent with
    // respect to Rust memory safety; errors are intentionally ignored in Drop.
    unsafe {
        libc::close(fd);
    }
}
