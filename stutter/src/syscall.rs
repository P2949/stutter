//! Safe wrappers around libc syscalls.

use std::{ffi::CStr, io, mem, os::unix::ffi::OsStrExt, path::Path};

#[path = "syscall/unix_diag/syscall.rs"]
mod unix_diag;
pub(crate) use unix_diag::unix_diag_dump;

#[cfg(feature = "wayland-probe")]
pub fn register_unix_stream_readiness(
    stream: std::os::unix::net::UnixStream,
) -> Result<
    tokio::io::unix::AsyncFd<std::os::unix::net::UnixStream>,
    tokio::io::unix::AsyncFdRegisterError<std::os::unix::net::UnixStream>,
> {
    // SAFETY: AsyncFd owns this UnixStream, which keeps its descriptor open and stable until drop.
    unsafe { tokio::io::unix::AsyncFd::register(stream) }
}

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

/// Opens a directory without following symlinks in any path component. Unsupported kernels fail
/// closed; callers must not fall back to a less constrained traversal for trusted artifacts.
pub fn open_dir_no_symlinks(path: &Path) -> io::Result<std::os::fd::OwnedFd> {
    use std::os::{fd::FromRawFd, unix::ffi::OsStrExt};
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))?;
    // SAFETY: open_how consists of integer fields and the kernel requires unknown fields to be 0.
    let mut how: libc::open_how = unsafe { std::mem::zeroed() };
    how.flags = (libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) as u64;
    how.resolve = libc::RESOLVE_NO_SYMLINKS | libc::RESOLVE_NO_MAGICLINKS;
    // SAFETY: c_path and how are valid for the duration of the syscall and its size is exact.
    let raw = unsafe {
        libc::syscall(
            libc::SYS_openat2,
            libc::AT_FDCWD,
            c_path.as_ptr(),
            &how,
            std::mem::size_of::<libc::open_how>(),
        )
    };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a successful openat2 returns a new owned file descriptor.
    Ok(unsafe { std::os::fd::OwnedFd::from_raw_fd(raw as libc::c_int) })
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

/// Creates one private (0700) child directory relative to `dir`; an existing name is an error.
pub fn mkdir_private_at(dir: std::os::fd::BorrowedFd<'_>, name: &str) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    let c_name =
        std::ffi::CString::new(name).map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))?;
    // SAFETY: `dir` is live and borrowed for this call; `c_name` is NUL terminated.
    let result = unsafe { libc::mkdirat(dir.as_raw_fd(), c_name.as_ptr(), 0o700) };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Opens a child directory relative to `dir` without following a symlink.
pub fn openat_dir_nofollow(
    dir: std::os::fd::BorrowedFd<'_>,
    name: &str,
) -> io::Result<std::os::fd::OwnedFd> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let c_name =
        std::ffi::CString::new(name).map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))?;
    // SAFETY: `dir` is live and borrowed for this call; `c_name` is NUL terminated.
    let raw = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            c_name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `raw` is a newly opened descriptor uniquely owned by this return value.
    Ok(unsafe { std::os::fd::OwnedFd::from_raw_fd(raw) })
}

/// Credentials recorded by the kernel for the connected Unix socket (`SO_PEERCRED`). For a
/// socket-activated listener these can identify the listener's creator rather than the process
/// that accepted the connection; callers must not assume otherwise.
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

/// Opens a pidfd for `pid`. The descriptor refers to that process only: if the process exits and
/// its number is reused, signals through the descriptor fail instead of reaching the new owner.
pub fn pidfd_open(pid: u32) -> io::Result<std::os::fd::OwnedFd> {
    use std::os::fd::FromRawFd;
    let pid = libc::pid_t::try_from(pid).map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))?;
    // SAFETY: pidfd_open(2) takes an integer pid and zero flags and returns a new descriptor.
    let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a successful pidfd_open returns a new descriptor owned by the caller.
    Ok(unsafe { std::os::fd::OwnedFd::from_raw_fd(raw as libc::c_int) })
}

/// Sends `signal` through a pidfd to exactly the process it was opened for. Signal 0 only checks
/// that the process still exists.
pub fn pidfd_send_signal(fd: std::os::fd::BorrowedFd<'_>, signal: i32) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    // SAFETY: pidfd_send_signal(2) with a valid descriptor, a plain signal number, no siginfo and
    // zero flags.
    let result = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            fd.as_raw_fd(),
            signal,
            std::ptr::null::<libc::siginfo_t>(),
            0u32,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Take a non-blocking exclusive `flock` on an open descriptor.
pub fn flock_exclusive_nonblocking(fd: std::os::fd::BorrowedFd<'_>) -> bool {
    use std::os::fd::AsRawFd;
    let attempt = || {
        // SAFETY: fd is a live borrowed descriptor; flock does not access user memory.
        unsafe { libc::flock(fd.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) == 0 }
    };
    // Test builds only: a child spawned by a concurrent test briefly holds a copy of every open
    // descriptor until it execs, which keeps an already-closed holder's lock alive. Retrying for a
    // moment keeps that transient from failing an unrelated test. Production never retries.
    #[cfg(test)]
    {
        for _ in 0..100 {
            if attempt() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        false
    }
    #[cfg(not(test))]
    attempt()
}

/// Exclusively create a new private (0600) file relative to an open directory.
pub fn create_new_at(dir: std::os::fd::BorrowedFd<'_>, name: &str) -> io::Result<std::fs::File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let name = std::ffi::CString::new(name)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "name contains NUL"))?;
    // SAFETY: name is NUL-terminated and dir is a live directory descriptor.
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            name.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: openat returned a fresh descriptor uniquely owned by the File.
    Ok(unsafe { std::fs::File::from_raw_fd(fd) })
}

/// Atomically rename within one open directory, failing if the target exists.
pub fn rename_noreplace_at(
    dir: std::os::fd::BorrowedFd<'_>,
    from: &str,
    to: &str,
) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    let invalid = |_| io::Error::new(io::ErrorKind::InvalidInput, "name contains NUL");
    let from = std::ffi::CString::new(from).map_err(invalid)?;
    let to = std::ffi::CString::new(to).map_err(invalid)?;
    // SAFETY: both names are NUL-terminated and relative to the same live directory descriptor.
    let rc = unsafe {
        libc::renameat2(
            dir.as_raw_fd(),
            from.as_ptr(),
            dir.as_raw_fd(),
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// chmod a name relative to an open directory without following a final symlink.
pub fn chmod_nofollow_at(
    dir: std::os::fd::BorrowedFd<'_>,
    name: &CStr,
    mode: u32,
) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    // SAFETY: name is NUL-terminated, dir is live; fchmodat2 with AT_SYMLINK_NOFOLLOW
    // refuses a substituted symlink.
    let rc = unsafe {
        libc::syscall(
            libc::SYS_fchmodat2,
            dir.as_raw_fd(),
            name.as_ptr(),
            mode as libc::mode_t,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// `linux/uinput.h` request numbers on 64-bit Linux (`_IO`/`_IOW` with type `U`).
const UI_DEV_CREATE: libc::c_ulong = 0x5501;
const UI_DEV_DESTROY: libc::c_ulong = 0x5502;
const UI_DEV_SETUP: libc::c_ulong = 0x405c_5503;
const UI_SET_EVBIT: libc::c_ulong = 0x4004_5564;
const UI_SET_KEYBIT: libc::c_ulong = 0x4004_5565;
/// `UI_GET_SYSNAME(64)`: `_IOR('U', 44, char[64])`, the kernel's own name for the device just created (`inputN`).
const UI_GET_SYSNAME_64: libc::c_ulong = 0x8040_552c;

/// `struct uinput_setup`: an `input_id`, an 80-byte name and `ff_effects_max`.
#[repr(C)]
struct UinputSetup {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
    name: [u8; 80],
    ff_effects_max: u32,
}

const _: () = assert!(mem::size_of::<UinputSetup>() == 92);

/// The request numbers, for a test to check against the kernel's encoding.
#[cfg(test)]
pub const fn uinput_requests() -> [libc::c_ulong; 6] {
    [
        UI_DEV_CREATE,
        UI_DEV_DESTROY,
        UI_DEV_SETUP,
        UI_SET_EVBIT,
        UI_SET_KEYBIT,
        UI_GET_SYSNAME_64,
    ]
}

/// Create a virtual keyboard on `node` that can press exactly `key_codes`. This registers a kernel-wide input device: the
/// caller must have proven the session it runs in is the isolated one. `UinputUnavailable` when the node cannot be opened,
/// `UinputSetupFailed` when the kernel refused a step.
pub fn uinput_keyboard(
    node: &std::path::Path,
    key_codes: &[u16],
    name: &str,
) -> Result<std::os::fd::OwnedFd, &'static str> {
    use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
    let file = std::fs::OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(node)
        .map_err(|_| "UinputUnavailable")?;
    let fd = std::os::fd::OwnedFd::from(file);
    let raw = fd.as_raw_fd();
    let by_value = |request: libc::c_ulong, argument: libc::c_int| {
        // SAFETY: `raw` is a descriptor this function owns; the request takes one int argument by value.
        (unsafe { libc::ioctl(raw, request, argument) }) == 0
    };
    let mut ok = by_value(UI_SET_EVBIT, 1) && by_value(UI_SET_EVBIT, 0);
    for code in key_codes {
        ok &= by_value(UI_SET_KEYBIT, libc::c_int::from(*code));
    }
    let mut setup = UinputSetup {
        bustype: 6,
        vendor: 0x5354,
        product: 0x0001,
        version: 1,
        name: [0; 80],
        ff_effects_max: 0,
    };
    for (slot, byte) in setup.name.iter_mut().zip(name.bytes().take(79)) {
        *slot = byte;
    }
    // SAFETY: `setup` is a live `uinput_setup` of exactly the size the request encodes.
    ok &= unsafe { libc::ioctl(raw, UI_DEV_SETUP, &raw const setup) } == 0;
    // SAFETY: the request takes no argument.
    ok &= unsafe { libc::ioctl(raw, UI_DEV_CREATE) } == 0;
    if ok { Ok(fd) } else { Err("UinputSetupFailed") }
}

/// The kernel's own name (`inputN`) for the device `uinput_keyboard` created, so a udev record can be tied to exactly that
/// device and no other.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "staged: the helper inside the isolated session asks for the name of the device it creates"
    )
)]
pub fn uinput_sysname(fd: &std::os::fd::OwnedFd) -> Result<String, &'static str> {
    use std::os::fd::AsRawFd;
    let mut name = [0u8; 64];
    // SAFETY: `name` is a live 64-byte buffer, exactly the size the request encodes, and the descriptor is live.
    let status = unsafe { libc::ioctl(fd.as_raw_fd(), UI_GET_SYSNAME_64, name.as_mut_ptr()) };
    if status != 0 {
        return Err("UinputSysnameUnavailable");
    }
    let end = name
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(name.len());
    String::from_utf8(name[..end].to_vec()).map_err(|_| "UinputSysnameUnavailable")
}

/// Destroy a device `uinput_keyboard` created (also what closing the descriptor does).
pub fn uinput_destroy(fd: &std::os::fd::OwnedFd) {
    use std::os::fd::AsRawFd;
    // SAFETY: the descriptor is live for this call; the request takes no argument.
    unsafe { libc::ioctl(fd.as_raw_fd(), UI_DEV_DESTROY) };
}
