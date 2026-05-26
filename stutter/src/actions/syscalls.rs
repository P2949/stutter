//! Safe wrappers around low-level action syscalls.

use std::{io, mem};

const IOPRIO_WHO_PROCESS: libc::c_int = 1;
const SCHED_FLAG_KEEP_POLICY: u64 = 0x08;
const SCHED_FLAG_KEEP_PARAMS: u64 = 0x10;
const SCHED_FLAG_UTIL_CLAMP_MIN: u64 = 0x20;
const SCHED_FLAG_UTIL_CLAMP_MAX: u64 = 0x40;
const UCLAMP_MIN_VALUE: u32 = 0;
const UCLAMP_MAX_VALUE: u32 = 1024;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct SchedAttr {
    size: u32,
    sched_policy: u32,
    sched_flags: u64,
    sched_nice: i32,
    sched_priority: u32,
    sched_runtime: u64,
    sched_deadline: u64,
    sched_period: u64,
    sched_util_min: u32,
    sched_util_max: u32,
}

impl Default for SchedAttr {
    fn default() -> Self {
        Self {
            size: mem::size_of::<SchedAttr>() as u32,
            sched_policy: 0,
            sched_flags: 0,
            sched_nice: 0,
            sched_priority: 0,
            sched_runtime: 0,
            sched_deadline: 0,
            sched_period: 0,
            sched_util_min: UCLAMP_MIN_VALUE,
            sched_util_max: UCLAMP_MAX_VALUE,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SchedUclamp {
    pub(crate) util_min: u32,
    pub(crate) util_max: u32,
}

pub(crate) fn setpriority_process(tid: u32, nice: i32) -> io::Result<()> {
    let rc = {
        // SAFETY: setpriority takes only integer arguments; OS errors are
        // reported through the return code and errno below.
        unsafe { libc::setpriority(libc::PRIO_PROCESS, tid as libc::id_t, nice as libc::c_int) }
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

pub(crate) fn ioprio_get_process(tid: u32) -> io::Result<i32> {
    // SAFETY: SYS_ioprio_get takes only integer arguments for process scope;
    // negative return values are converted to OS errors below.
    let rc = unsafe { libc::syscall(libc::SYS_ioprio_get, IOPRIO_WHO_PROCESS, tid as libc::c_int) };
    if rc >= 0 {
        Ok(rc as i32)
    } else {
        Err(io::Error::last_os_error())
    }
}

pub(crate) fn ioprio_set_process(tid: u32, encoded_ioprio: i32) -> io::Result<()> {
    // SAFETY: SYS_ioprio_set takes only integer arguments for process scope;
    // the caller validates the encoded ioprio value before invoking this.
    let rc = unsafe {
        libc::syscall(
            libc::SYS_ioprio_set,
            IOPRIO_WHO_PROCESS,
            tid as libc::c_int,
            encoded_ioprio as libc::c_int,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

pub(crate) fn sched_getattr(tid: u32) -> io::Result<SchedUclamp> {
    let mut attr = SchedAttr::default();
    // SAFETY: attr points to writable sched_attr-compatible storage and the
    // kernel initializes it for the given task on success.
    let rc = unsafe {
        libc::syscall(
            libc::SYS_sched_getattr,
            tid as libc::pid_t,
            &mut attr as *mut SchedAttr,
            mem::size_of::<SchedAttr>() as u32,
            0u32,
        )
    };
    if rc == 0 {
        Ok(SchedUclamp {
            util_min: attr.sched_util_min,
            util_max: attr.sched_util_max,
        })
    } else {
        Err(io::Error::last_os_error())
    }
}

pub(crate) fn sched_setattr(tid: u32, values: SchedUclamp) -> io::Result<()> {
    let mut attr = SchedAttr {
        sched_flags: SCHED_FLAG_KEEP_POLICY
            | SCHED_FLAG_KEEP_PARAMS
            | SCHED_FLAG_UTIL_CLAMP_MIN
            | SCHED_FLAG_UTIL_CLAMP_MAX,
        sched_util_min: values.util_min,
        sched_util_max: values.util_max,
        ..SchedAttr::default()
    };

    // SAFETY: attr is a fully initialized sched_attr-compatible value and the
    // syscall arguments are plain integers or pointers valid for this call.
    let rc = unsafe {
        libc::syscall(
            libc::SYS_sched_setattr,
            tid as libc::pid_t,
            &mut attr as *mut SchedAttr,
            0u32,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
