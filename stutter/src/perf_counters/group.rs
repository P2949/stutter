use std::{io, mem, os::fd::RawFd};

use log::warn;
use stutter_core::ids::Tid;

use super::{limits::CpuPerfConfig, sample::CpuPerfDelta, syscall};

const PERF_TYPE_HARDWARE: u32 = 0;
const PERF_COUNT_HW_CPU_CYCLES: u64 = 0;
const PERF_COUNT_HW_INSTRUCTIONS: u64 = 1;
const PERF_COUNT_HW_CACHE_REFERENCES: u64 = 2;
const PERF_COUNT_HW_CACHE_MISSES: u64 = 3;
const PERF_COUNT_HW_BRANCH_INSTRUCTIONS: u64 = 4;
const PERF_COUNT_HW_BRANCH_MISSES: u64 = 5;

const PERF_FORMAT_TOTAL_TIME_ENABLED: u64 = 1 << 0;
const PERF_FORMAT_TOTAL_TIME_RUNNING: u64 = 1 << 1;
const PERF_FORMAT_GROUP: u64 = 1 << 3;

const PERF_FLAG_FD_CLOEXEC: u64 = 1 << 3;

const PERF_EVENT_IOC_ENABLE: libc::c_ulong = 0x2400;
const PERF_EVENT_IOC_DISABLE: libc::c_ulong = 0x2401;
const PERF_EVENT_IOC_RESET: libc::c_ulong = 0x2403;
const PERF_IOC_FLAG_GROUP: libc::c_ulong = 1;

const PERF_ATTR_DISABLED: u64 = 1 << 0;
const PERF_ATTR_EXCLUDE_KERNEL: u64 = 1 << 5;
const PERF_ATTR_EXCLUDE_HV: u64 = 1 << 6;
const PERF_ATTR_EXCLUDE_IDLE: u64 = 1 << 7;

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct PerfEventAttr {
    type_: u32,
    size: u32,
    config: u64,
    sample_period: u64,
    sample_type: u64,
    read_format: u64,
    flags: u64,
    wakeup_events: u32,
    bp_type: u32,
    bp_addr: u64,
}

impl Default for PerfEventAttr {
    fn default() -> Self {
        Self {
            type_: 0,
            size: mem::size_of::<Self>() as u32,
            config: 0,
            sample_period: 0,
            sample_type: 0,
            read_format: 0,
            flags: 0,
            wakeup_events: 0,
            bp_type: 0,
            bp_addr: 0,
        }
    }
}

pub(super) struct PerfCounterGroup {
    leader: OwnedFd,
    siblings: Vec<OwnedFd>,
    events: Vec<PerfEventKind>,
    intervals: super::interval::IntervalTracker,
    unsupported_events: Vec<String>,
}

impl PerfCounterGroup {
    pub(super) fn open(tid: Tid, config: &CpuPerfConfig) -> Result<Self, PerfOpenError> {
        let tid = tid.as_u32() as i32;
        let mut leader_attr =
            perf_event_attr(PERF_COUNT_HW_CPU_CYCLES, true, config.include_kernel);
        let leader_fd =
            perf_event_open(&mut leader_attr, tid, -1, -1).map_err(PerfOpenError::from_io)?;
        let leader = OwnedFd::new(leader_fd);

        let mut group = Self {
            leader,
            siblings: Vec::new(),
            events: vec![PerfEventKind::Cycles],
            intervals: super::interval::IntervalTracker::new(1),
            unsupported_events: Vec::new(),
        };

        group
            .open_sibling(
                tid,
                PERF_COUNT_HW_INSTRUCTIONS,
                PerfEventKind::Instructions,
                config,
            )
            .map_err(PerfOpenError::from_io)?;
        group
            .open_sibling(
                tid,
                PERF_COUNT_HW_CACHE_MISSES,
                PerfEventKind::CacheMisses,
                config,
            )
            .map_err(PerfOpenError::from_io)?;

        if config.collect_cache_refs
            && let Err(err) = group.open_sibling(
                tid,
                PERF_COUNT_HW_CACHE_REFERENCES,
                PerfEventKind::CacheReferences,
                config,
            )
        {
            if matches!(err.raw_os_error(), Some(libc::EINVAL | libc::ENOENT)) {
                group.unsupported_events.push("cache_references".into());
                warn!(
                    "cpu_perf_cache_references_unavailable tid={} err={}",
                    tid, err
                );
            } else {
                return Err(PerfOpenError::from_io(err));
            }
        }

        if config.collect_branches {
            for (value, event, name) in [
                (
                    PERF_COUNT_HW_BRANCH_INSTRUCTIONS,
                    PerfEventKind::Branches,
                    "branches",
                ),
                (
                    PERF_COUNT_HW_BRANCH_MISSES,
                    PerfEventKind::BranchMisses,
                    "branch_misses",
                ),
            ] {
                if let Err(err) = group.open_sibling(tid, value, event, config) {
                    if matches!(
                        err.raw_os_error(),
                        Some(libc::EINVAL | libc::ENOENT | libc::EOPNOTSUPP | libc::ENODEV)
                    ) {
                        warn!(
                            "cpu_perf_branch_event_unavailable tid={tid} config={value} err={err}"
                        );
                        group.unsupported_events.push(name.into());
                    } else {
                        return Err(PerfOpenError::from_io(err));
                    }
                }
            }
        }
        group
            .reset_and_enable()
            .map_err(|err| PerfOpenError::from_message(err.to_string()))?;
        group.intervals = super::interval::IntervalTracker::new(group.events.len());
        Ok(group)
    }

    fn open_sibling(
        &mut self,
        tid: i32,
        config_value: u64,
        event: PerfEventKind,
        config: &CpuPerfConfig,
    ) -> io::Result<()> {
        let mut attr = perf_event_attr(config_value, false, config.include_kernel);
        let fd = perf_event_open(&mut attr, tid, -1, self.leader.fd)?;
        self.siblings.push(OwnedFd::new(fd));
        self.events.push(event);
        Ok(())
    }

    pub(super) fn sample_interval(&mut self) -> Result<CpuPerfDelta, String> {
        let expected = self.events.len();
        let mut values = [0u64; super::interval::WORDS];
        let expected_bytes = (3 + expected) * mem::size_of::<u64>();
        let read_bytes = syscall::read_counters(self.leader.fd, &mut values, expected_bytes)
            .map_err(|err| {
                self.intervals.invalidate();
                format!("read failed: {err}")
            })?;
        if read_bytes as usize != expected_bytes {
            self.intervals.invalidate();
            return Err(format!(
                "short perf group read: got {} bytes, expected {}",
                read_bytes, expected_bytes
            ));
        }

        let values = self.intervals.interval(values, expected)?;
        Ok(decode_interval(
            &self.events,
            values,
            &self.unsupported_events,
        ))
    }

    fn reset_and_enable(&self) -> io::Result<()> {
        perf_ioctl(self.leader.fd, PERF_EVENT_IOC_DISABLE, PERF_IOC_FLAG_GROUP)?;
        perf_ioctl(self.leader.fd, PERF_EVENT_IOC_RESET, PERF_IOC_FLAG_GROUP)?;
        perf_ioctl(self.leader.fd, PERF_EVENT_IOC_ENABLE, PERF_IOC_FLAG_GROUP)?;
        Ok(())
    }
}

fn decode_interval(
    events: &[PerfEventKind],
    values: [u64; super::interval::WORDS],
    unsupported: &[String],
) -> CpuPerfDelta {
    let time_enabled = values[1];
    let time_running = values[2];

    let multiplexed = time_enabled != time_running && time_enabled > 0;
    let mut scaled = false;
    let mut delta = CpuPerfDelta {
        time_enabled_ns: Some(time_enabled),
        time_running_ns: Some(time_running),
        multiplexed,
        unsupported_events: unsupported.to_vec(),
        ..Default::default()
    };

    for (idx, kind) in events.iter().copied().enumerate() {
        let raw = values[3 + idx];
        let value = super::sample::scale(raw, time_enabled, time_running);
        if value.is_none() {
            delta.unavailable_reason =
                Some("perf interval did not run or scaled count overflowed".into());
        }
        if multiplexed && value.is_some() {
            scaled = true;
        }
        match kind {
            PerfEventKind::Cycles => delta.cycles = value,
            PerfEventKind::Instructions => delta.instructions = value,
            PerfEventKind::CacheMisses => delta.cache_misses = value,
            PerfEventKind::CacheReferences => delta.cache_references = value,
            PerfEventKind::Branches => delta.branches = value,
            PerfEventKind::BranchMisses => delta.branch_misses = value,
        }
    }

    super::sample::apply_derived_metrics(&mut delta);
    delta.scaled = scaled;

    delta
}

#[derive(Clone, Copy)]
enum PerfEventKind {
    Cycles,
    Instructions,
    CacheMisses,
    CacheReferences,
    Branches,
    BranchMisses,
}

pub(super) struct OwnedFd {
    fd: RawFd,
}

impl OwnedFd {
    pub(super) fn new(fd: RawFd) -> Self {
        Self { fd }
    }
}

impl Drop for OwnedFd {
    fn drop(&mut self) {
        syscall::close_fd(self.fd);
    }
}

pub(super) struct PerfOpenError {
    pub(super) message: String,
    errno: Option<i32>,
}

impl PerfOpenError {
    pub(super) fn from_io(err: io::Error) -> Self {
        Self {
            message: err.to_string(),
            errno: err.raw_os_error(),
        }
    }

    pub(super) fn from_message(message: String) -> Self {
        Self {
            message,
            errno: None,
        }
    }

    pub(super) fn is_permission_denied(&self) -> bool {
        matches!(self.errno, Some(libc::EACCES | libc::EPERM))
    }

    pub(super) fn is_task_gone(&self) -> bool {
        matches!(self.errno, Some(libc::ESRCH))
    }
}

pub(super) fn perf_event_attr(config: u64, disabled: bool, include_kernel: bool) -> PerfEventAttr {
    let mut attr = PerfEventAttr {
        type_: PERF_TYPE_HARDWARE,
        config,
        sample_period: 0,
        sample_type: 0,
        read_format: PERF_FORMAT_GROUP
            | PERF_FORMAT_TOTAL_TIME_ENABLED
            | PERF_FORMAT_TOTAL_TIME_RUNNING,
        ..Default::default()
    };

    attr.flags |= PERF_ATTR_EXCLUDE_IDLE;
    if disabled {
        attr.flags |= PERF_ATTR_DISABLED;
    }
    if !include_kernel {
        attr.flags |= PERF_ATTR_EXCLUDE_KERNEL | PERF_ATTR_EXCLUDE_HV;
    }
    attr
}

pub(super) fn perf_event_open(
    attr: &mut PerfEventAttr,
    pid: libc::pid_t,
    cpu: libc::c_int,
    group_fd: RawFd,
) -> io::Result<RawFd> {
    syscall::perf_event_open(attr, pid, cpu, group_fd, PERF_FLAG_FD_CLOEXEC)
}

fn perf_ioctl(fd: RawFd, request: libc::c_ulong, arg: libc::c_ulong) -> io::Result<()> {
    syscall::perf_ioctl(fd, request, arg)
}

pub(crate) fn try_open_disabled_cycles_current_thread(include_kernel: bool) -> io::Result<()> {
    let tid = crate::syscall::gettid() as i32;
    let mut attr = perf_event_attr(PERF_COUNT_HW_CPU_CYCLES, true, include_kernel);
    let fd = perf_event_open(&mut attr, tid, -1, -1)?;
    let _owned = OwnedFd::new(fd);
    Ok(())
}

#[cfg(test)]
#[path = "group_tests.rs"]
mod tests;
