use std::{collections::BTreeMap, path::Path};

use super::{
    model::{DoctorCheck, DoctorStatus},
    utils::{format_rlimit_bytes, read_trimmed, yes_no},
};

pub(crate) fn ebpf_build_check() -> DoctorCheck {
    DoctorCheck {
        name: "ebpf_build".to_owned(),
        status: DoctorStatus::Pass,
        message: "binary started; eBPF object was embedded at build time".to_owned(),
        details: BTreeMap::new(),
    }
}

pub(crate) fn ebpf_runtime_permission_check() -> DoctorCheck {
    let euid = crate::syscall::geteuid();
    let rlimit = crate::syscall::get_memlock_rlimit().ok();
    let unprivileged_bpf_disabled =
        read_trimmed(Path::new("/proc/sys/kernel/unprivileged_bpf_disabled"));

    ebpf_runtime_permission_check_from_parts(euid as libc::uid_t, rlimit, unprivileged_bpf_disabled)
}

pub(crate) fn ebpf_runtime_permission_check_from_parts(
    euid: libc::uid_t,
    memlock: Option<(u64, u64)>,
    unprivileged_bpf_disabled: Result<Option<String>, String>,
) -> DoctorCheck {
    let mut details = BTreeMap::new();
    details.insert("effective_uid".to_owned(), euid.to_string());
    details.insert("is_root".to_owned(), yes_no(euid == 0));

    match memlock {
        Some((soft, hard)) => {
            details.insert(
                "rlimit_memlock_soft_bytes".to_owned(),
                format_rlimit_bytes(soft),
            );
            details.insert(
                "rlimit_memlock_hard_bytes".to_owned(),
                format_rlimit_bytes(hard),
            );
        }
        None => {
            details.insert("rlimit_memlock_soft_bytes".to_owned(), "unknown".to_owned());
            details.insert("rlimit_memlock_hard_bytes".to_owned(), "unknown".to_owned());
        }
    }

    match unprivileged_bpf_disabled {
        Ok(Some(val)) => {
            details.insert("unprivileged_bpf_disabled".to_owned(), val);
        }
        Ok(None) => {
            details.insert("unprivileged_bpf_disabled".to_owned(), "missing".to_owned());
        }
        Err(err) => {
            details.insert("unprivileged_bpf_disabled_error".to_owned(), err);
        }
    }

    let (status, message) = if euid == 0 {
        (
            DoctorStatus::Pass,
            "process is running as root; eBPF recording should have the required runtime privileges"
                .to_owned(),
        )
    } else {
        (
            DoctorStatus::Warn,
            "recording likely requires root or CAP_BPF/CAP_PERFMON/CAP_SYS_RESOURCE; build as your normal user, then run the built stutter binary with doas/sudo"
                .to_owned(),
        )
    };

    DoctorCheck {
        name: "ebpf_runtime_permissions".to_owned(),
        status,
        message,
        details,
    }
}
