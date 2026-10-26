//! The only module with `unsafe`: thin, checked wrappers over Linux syscalls. Every function
//! is async-signal-safe (plain syscalls, no allocation) so they can run between `fork` and
//! `exec` in a `pre_exec` hook.
use std::{
    io,
    os::{fd::AsRawFd, unix::net::UnixStream},
};

const CAPABILITY_VERSION_3: u32 = 0x2008_0522;

#[repr(C)]
struct CapHeader {
    version: u32,
    pid: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CapData {
    effective: u32,
    permitted: u32,
    inheritable: u32,
}

/// Capability sets as 64-bit masks (bit N = capability N).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CapSets {
    pub effective: u64,
    pub permitted: u64,
    pub inheritable: u64,
}

fn check(result: libc::c_long) -> io::Result<libc::c_long> {
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(result)
    }
}

/// Peer identity of a connected Unix socket, as reported by the kernel (`SO_PEERCRED`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PeerCredentials {
    pub pid: u32,
    pub uid: u32,
    pub gid: u32,
}

pub fn peer_credentials(stream: &UnixStream) -> io::Result<PeerCredentials> {
    let mut credentials = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: `credentials` and `length` are valid for writes of the sizes passed, the fd is
    // owned by `stream` for the duration of the call, and SO_PEERCRED only fills a `ucred`.
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            std::ptr::addr_of_mut!(credentials).cast(),
            &mut length,
        )
    };
    check(libc::c_long::from(result))?;
    Ok(PeerCredentials {
        pid: credentials.pid as u32,
        uid: credentials.uid,
        gid: credentials.gid,
    })
}

pub fn set_no_new_privs() -> io::Result<()> {
    // SAFETY: prctl(PR_SET_NO_NEW_PRIVS, 1) takes no pointers.
    check(unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) }.into()).map(drop)
}

pub fn no_new_privs() -> io::Result<bool> {
    // SAFETY: prctl(PR_GET_NO_NEW_PRIVS) takes no pointers and returns 0 or 1.
    let value = check(unsafe { libc::prctl(libc::PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0) }.into())?;
    Ok(value == 1)
}

pub fn clear_ambient() -> io::Result<()> {
    // SAFETY: prctl(PR_CAP_AMBIENT, CLEAR_ALL) takes no pointers.
    check(
        unsafe {
            libc::prctl(
                libc::PR_CAP_AMBIENT,
                libc::PR_CAP_AMBIENT_CLEAR_ALL,
                0,
                0,
                0,
            )
        }
        .into(),
    )
    .map(drop)
}

pub fn raise_ambient(capability: u32) -> io::Result<()> {
    // SAFETY: prctl(PR_CAP_AMBIENT, RAISE, cap) takes only integers.
    check(
        unsafe {
            libc::prctl(
                libc::PR_CAP_AMBIENT,
                libc::PR_CAP_AMBIENT_RAISE,
                libc::c_ulong::from(capability),
                0,
                0,
            )
        }
        .into(),
    )
    .map(drop)
}

pub fn keep_caps_across_setuid(keep: bool) -> io::Result<()> {
    // SAFETY: prctl(PR_SET_KEEPCAPS, flag) takes only integers.
    check(unsafe { libc::prctl(libc::PR_SET_KEEPCAPS, libc::c_ulong::from(keep), 0, 0, 0) }.into())
        .map(drop)
}

/// Drops `capability` from the bounding set (needs CAP_SETPCAP).
pub fn drop_bounding(capability: u32) -> io::Result<()> {
    // SAFETY: prctl(PR_CAPBSET_DROP, cap) takes only integers.
    check(
        unsafe {
            libc::prctl(
                libc::PR_CAPBSET_DROP,
                libc::c_ulong::from(capability),
                0,
                0,
                0,
            )
        }
        .into(),
    )
    .map(drop)
}

pub fn set_capabilities(sets: CapSets) -> io::Result<()> {
    let header = CapHeader {
        version: CAPABILITY_VERSION_3,
        pid: 0,
    };
    let split = |mask: u64| [mask as u32, (mask >> 32) as u32];
    let (effective, permitted, inheritable) = (
        split(sets.effective),
        split(sets.permitted),
        split(sets.inheritable),
    );
    let data = [
        CapData {
            effective: effective[0],
            permitted: permitted[0],
            inheritable: inheritable[0],
        },
        CapData {
            effective: effective[1],
            permitted: permitted[1],
            inheritable: inheritable[1],
        },
    ];
    // SAFETY: `header` and `data` are valid, correctly laid out v3 capability structures (one
    // header, two data words) that outlive the call; pid 0 means the calling thread.
    check(unsafe { libc::syscall(libc::SYS_capset, &header, data.as_ptr()) }).map(drop)
}

pub fn capabilities() -> io::Result<CapSets> {
    let mut header = CapHeader {
        version: CAPABILITY_VERSION_3,
        pid: 0,
    };
    let mut data = [CapData::default(); 2];
    // SAFETY: `header` and `data` are valid, writable v3 capability structures (one header,
    // two data words) that outlive the call; pid 0 means the calling thread.
    check(unsafe { libc::syscall(libc::SYS_capget, &mut header, data.as_mut_ptr()) })?;
    let join = |low: u32, high: u32| u64::from(low) | (u64::from(high) << 32);
    Ok(CapSets {
        effective: join(data[0].effective, data[1].effective),
        permitted: join(data[0].permitted, data[1].permitted),
        inheritable: join(data[0].inheritable, data[1].inheritable),
    })
}

/// Drops supplementary groups, then switches real/effective/saved gid and uid.
pub fn become_user(uid: u32, gid: u32) -> io::Result<()> {
    // SAFETY: setgroups(0, NULL) clears the list and dereferences nothing.
    check(unsafe { libc::setgroups(0, std::ptr::null()) }.into())?;
    // SAFETY: setresgid/setresuid take only integers.
    check(unsafe { libc::setresgid(gid, gid, gid) }.into())?;
    // SAFETY: as above.
    check(unsafe { libc::setresuid(uid, uid, uid) }.into()).map(drop)
}

pub fn pin_to_cpu(cpu: u32) -> io::Result<()> {
    // SAFETY: `set` is a zero-initialised cpu_set_t; CPU_ZERO/CPU_SET only write inside it and
    // sched_setaffinity reads `size_of::<cpu_set_t>()` bytes from it.
    unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        libc::CPU_ZERO(&mut set);
        libc::CPU_SET(cpu as usize, &mut set);
        check(libc::c_long::from(libc::sched_setaffinity(
            0,
            std::mem::size_of::<libc::cpu_set_t>(),
            &set,
        )))
        .map(drop)
    }
}

pub fn set_fifo_priority(priority: i32) -> io::Result<()> {
    let param = libc::sched_param {
        sched_priority: priority,
    };
    // SAFETY: `param` is a valid sched_param; pid 0 targets the calling thread.
    check(libc::c_long::from(unsafe {
        libc::sched_setscheduler(0, libc::SCHED_FIFO, &param)
    }))
    .map(drop)
}

pub fn effective_uid() -> u32 {
    // SAFETY: geteuid takes no arguments and cannot fail.
    unsafe { libc::geteuid() }
}

/// Writes `0` (the calling process) into the given `cgroup.procs`. Raw syscalls only, so it is
/// safe between fork and exec.
pub fn join_cgroup(procs_file: &std::ffi::CStr) -> io::Result<()> {
    // SAFETY: `procs_file` is a valid NUL-terminated path; the buffer outlives the call.
    let fd = unsafe { libc::open(procs_file.as_ptr(), libc::O_WRONLY | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let zero = b"0";
    // SAFETY: `fd` is open and `zero` is a valid one-byte buffer.
    let written = unsafe { libc::write(fd, zero.as_ptr().cast(), zero.len()) };
    let result = if written == 1 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    };
    // SAFETY: `fd` was opened above and is closed exactly once.
    unsafe { libc::close(fd) };
    result
}
