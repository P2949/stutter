//! eBPF memlock limit policy.

use crate::ebpf::{memory::format_optional_bytes, model::MemlockPolicyReport};

pub(crate) fn locked_memory_limit_bytes() -> Option<u64> {
    read_memlock_rlimit()
        .ok()
        .and_then(|rlim| memlock_limit_bytes_from_rlim(rlim.rlim_cur))
}

pub(crate) fn memlock_limit_bytes_from_rlim(value: libc::rlim_t) -> Option<u64> {
    if value == libc::RLIM_INFINITY {
        None
    } else {
        Some(value)
    }
}

fn read_memlock_rlimit() -> std::io::Result<libc::rlimit> {
    let mut rlim = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };

    let ret = unsafe { libc::getrlimit(libc::RLIMIT_MEMLOCK, &mut rlim) };
    if ret == 0 {
        Ok(rlim)
    } else {
        Err(std::io::Error::last_os_error())
    }
}

pub(crate) fn raise_memlock_limit() -> MemlockPolicyReport {
    let before = match read_memlock_rlimit() {
        Ok(rlim) => rlim,
        Err(err) => {
            return MemlockPolicyReport {
                before_limit_bytes: None,
                after_limit_bytes: locked_memory_limit_bytes(),
                raise_attempted: false,
                raise_succeeded: false,
                raise_error: Some(format!("failed to read RLIMIT_MEMLOCK before raise: {err}")),
            };
        }
    };

    let before_limit_bytes = memlock_limit_bytes_from_rlim(before.rlim_cur);
    if before.rlim_cur == libc::RLIM_INFINITY {
        return MemlockPolicyReport {
            before_limit_bytes,
            after_limit_bytes: before_limit_bytes,
            raise_attempted: false,
            raise_succeeded: false,
            raise_error: None,
        };
    }

    // Existing policy: try to make memlock unlimited for eBPF loading. If this
    // fails, continue startup and size maps from the effective post-attempt
    // limit so low-memlock systems remain conservative instead of aborting here.
    let unlimited = libc::rlimit {
        rlim_cur: libc::RLIM_INFINITY,
        rlim_max: libc::RLIM_INFINITY,
    };
    let ret = unsafe { libc::setrlimit(libc::RLIMIT_MEMLOCK, &unlimited) };
    let raise_succeeded = ret == 0;
    let raise_error = if raise_succeeded {
        None
    } else {
        Some(format!(
            "failed to raise RLIMIT_MEMLOCK to unlimited: {}",
            std::io::Error::last_os_error()
        ))
    };

    MemlockPolicyReport {
        before_limit_bytes,
        after_limit_bytes: locked_memory_limit_bytes(),
        raise_attempted: true,
        raise_succeeded,
        raise_error,
    }
}

pub(crate) fn log_memlock_policy_report(report: &MemlockPolicyReport) {
    let raise_error = report.raise_error.as_deref().unwrap_or("none");

    if report.raise_error.is_some() {
        log::warn!(
            "memlock_policy before_limit={} after_limit={} raise_attempted={} raise_succeeded={} raise_error={}",
            format_optional_bytes(report.before_limit_bytes),
            format_optional_bytes(report.after_limit_bytes),
            report.raise_attempted,
            report.raise_succeeded,
            raise_error,
        );
    } else {
        log::info!(
            "memlock_policy before_limit={} after_limit={} raise_attempted={} raise_succeeded={} raise_error={}",
            format_optional_bytes(report.before_limit_bytes),
            format_optional_bytes(report.after_limit_bytes),
            report.raise_attempted,
            report.raise_succeeded,
            raise_error,
        );
    }
}
