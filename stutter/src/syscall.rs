//! Safe wrappers around libc syscalls.

use std::{ffi::CStr, io, mem, os::unix::ffi::OsStrExt, path::Path};

pub fn clock_ticks_per_second() -> u64 {
    // SAFETY: _SC_CLK_TCK is a valid sysconf name and has no side effects.
    let value = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if value <= 0 {
        100 // fallback
    } else {
        value as u64
    }
}

pub fn get_avphys_pages() -> Option<u64> {
    // SAFETY: _SC_AVPHYS_PAGES is a valid sysconf name and has no side effects.
    let value = unsafe { libc::sysconf(libc::_SC_AVPHYS_PAGES) };
    if value <= 0 { None } else { Some(value as u64) }
}

pub fn get_pagesize() -> u64 {
    // SAFETY: _SC_PAGESIZE is a valid sysconf name and has no side effects.
    let value = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if value <= 0 {
        4096 // fallback
    } else {
        value as u64
    }
}

pub struct DiskSpace {
    pub free_bytes: u64,
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
            free_bytes: stat.f_bfree.saturating_mul(stat.f_frsize),
        })
    } else {
        Err(io::Error::last_os_error())
    }
}

pub fn geteuid() -> u32 {
    // SAFETY: geteuid is always safe to call.
    unsafe { libc::geteuid() as u32 }
}

pub fn get_memlock_rlimit() -> io::Result<(u64, u64)> {
    let mut limit = mem::MaybeUninit::<libc::rlimit>::uninit();
    // SAFETY: limit points to valid uninitialized memory.
    let rc = unsafe { libc::getrlimit(libc::RLIMIT_MEMLOCK, limit.as_mut_ptr()) };
    if rc == 0 {
        // SAFETY: getrlimit initialized the struct on success.
        let limit = unsafe { limit.assume_init() };
        Ok((limit.rlim_cur, limit.rlim_max))
    } else {
        Err(io::Error::last_os_error())
    }
}

pub fn set_memlock_rlimit(cur: u64, max: u64) -> io::Result<()> {
    let limit = libc::rlimit {
        rlim_cur: cur,
        rlim_max: max,
    };
    // SAFETY: limit points to a valid rlimit struct.
    let rc = unsafe { libc::setrlimit(libc::RLIMIT_MEMLOCK, &limit) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

pub fn gettid() -> u32 {
    // SAFETY: SYS_gettid is always safe to call.
    unsafe { libc::syscall(libc::SYS_gettid) as u32 }
}

pub fn get_nofile_rlimit_cur() -> std::io::Result<u64> {
    let mut limit = std::mem::MaybeUninit::<libc::rlimit>::uninit();
    // SAFETY: limit points to valid uninitialized memory.
    let rc = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, limit.as_mut_ptr()) };
    if rc == 0 {
        // SAFETY: getrlimit initialized the struct on success.
        let limit = unsafe { limit.assume_init() };
        Ok(limit.rlim_cur)
    } else {
        Err(std::io::Error::last_os_error())
    }
}

pub fn clock_gettime_ns(clock_id: libc::clockid_t) -> io::Result<u64> {
    let mut timespec = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };

    // SAFETY: clock_gettime writes to the provided valid timespec pointer and
    // does not retain it after the call.
    let result = unsafe { libc::clock_gettime(clock_id, &mut timespec) };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }

    timespec_to_ns(timespec).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "clock_gettime returned invalid timespec",
        )
    })
}

pub fn uname_release() -> io::Result<String> {
    let mut uts = mem::MaybeUninit::<libc::utsname>::uninit();
    // SAFETY: uts points to valid uninitialized memory for libc to fill.
    if unsafe { libc::uname(uts.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: uname initialized the struct on success.
    let uts = unsafe { uts.assume_init() };
    // SAFETY: the release field is a null-terminated C string from uname.
    let release = unsafe { CStr::from_ptr(uts.release.as_ptr()) };
    Ok(release.to_string_lossy().into_owned())
}

fn timespec_to_ns(timespec: libc::timespec) -> Option<u64> {
    if timespec.tv_sec < 0 || timespec.tv_nsec < 0 {
        return None;
    }

    let seconds = u64::try_from(timespec.tv_sec).ok()?;
    let nanos = u64::try_from(timespec.tv_nsec).ok()?;
    if nanos >= 1_000_000_000 {
        return None;
    }

    seconds.checked_mul(1_000_000_000)?.checked_add(nanos)
}

/// Lets this process and every process it starts restore a lower niceness than they currently
/// have. The kernel checks the *target* task's `RLIMIT_NICE` (0 for an ordinary user), so a
/// validation harness raises its own limit with `CAP_SYS_RESOURCE` before it spawns the disposable
/// workload and contention it will later un-demote. Production restores through the privileged
/// worker instead.
pub fn raise_nice_rlimit_for_validation() -> io::Result<()> {
    let limit = libc::rlimit {
        rlim_cur: 40,
        rlim_max: 40,
    };
    // SAFETY: `limit` is a valid, initialised rlimit and RLIMIT_NICE a valid resource.
    let rc = unsafe { libc::setrlimit(libc::RLIMIT_NICE, &limit) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Opens `path` as a directory without following a final symlink.
pub fn open_dir_nofollow(path: &Path) -> io::Result<std::os::fd::OwnedFd> {
    use std::os::{fd::FromRawFd, unix::ffi::OsStrExt};
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))?;
    // SAFETY: `c_path` is a valid NUL-terminated string that outlives the call.
    let raw = unsafe {
        libc::open(
            c_path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `raw` is a freshly opened descriptor that nothing else owns.
    Ok(unsafe { std::os::fd::OwnedFd::from_raw_fd(raw) })
}

/// Opens `name` relative to the directory descriptor `dir` for reading, without following a
/// symlink and without blocking on a FIFO.
pub fn openat_nofollow(dir: std::os::fd::BorrowedFd<'_>, name: &str) -> io::Result<std::fs::File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let c_name =
        std::ffi::CString::new(name).map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))?;
    // SAFETY: `dir` is a live descriptor borrowed for this call and `c_name` is a valid
    // NUL-terminated string that outlives it.
    let raw = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            c_name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
        )
    };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `raw` is a freshly opened descriptor that nothing else owns.
    Ok(unsafe { std::fs::File::from_raw_fd(raw) })
}

/// Credentials of the process on the other end of a connected Unix socket, as recorded by the
/// kernel when the connection was made (`SO_PEERCRED`).
pub fn unix_peer_credentials(
    socket: &std::os::unix::net::UnixStream,
) -> io::Result<(u32, u32, u32)> {
    use std::os::fd::AsRawFd;
    let mut cred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: the descriptor is live for this call and `cred`/`len` describe a writable
    // `ucred` buffer of exactly `len` bytes.
    let status = unsafe {
        libc::getsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&raw mut cred).cast(),
            &raw mut len,
        )
    };
    if status != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((cred.pid as u32, cred.uid, cred.gid))
}

static PENDING_SIGNAL: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

extern "C" fn record_signal(signal: libc::c_int) {
    PENDING_SIGNAL.store(signal, std::sync::atomic::Ordering::SeqCst);
}

/// Records SIGINT, SIGTERM and SIGHUP instead of dying, so a wrapper can forward them to its
/// child and report the child's own status.
pub fn catch_termination_signals() {
    for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        // SAFETY: `record_signal` only stores to an atomic, which is async-signal-safe.
        unsafe { libc::signal(signal, record_signal as *const () as libc::sighandler_t) };
    }
}

/// The last caught termination signal, cleared by the call.
pub fn take_caught_signal() -> Option<i32> {
    match PENDING_SIGNAL.swap(0, std::sync::atomic::Ordering::SeqCst) {
        0 => None,
        signal => Some(signal),
    }
}

/// Sends `signal` to exactly process `pid`.
pub fn send_signal(pid: u32, signal: i32) -> io::Result<()> {
    let pid = libc::pid_t::try_from(pid).map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))?;
    // SAFETY: plain kill(2) to one explicit, positive pid.
    if unsafe { libc::kill(pid, signal) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
