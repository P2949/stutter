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
