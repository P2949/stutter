use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io, mem,
    os::fd::RawFd,
};

use log::warn;
use serde::{Deserialize, Serialize};

use crate::{
    metrics::TaskStats,
    process_tree::{TaskClass, TaskInfo},
};

const PERF_TYPE_HARDWARE: u32 = 0;
const PERF_COUNT_HW_CPU_CYCLES: u64 = 0;
const PERF_COUNT_HW_INSTRUCTIONS: u64 = 1;
const PERF_COUNT_HW_CACHE_REFERENCES: u64 = 2;
const PERF_COUNT_HW_CACHE_MISSES: u64 = 3;

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

#[derive(Clone, Debug)]
pub struct CpuPerfConfig {
    pub include_kernel: bool,
    pub max_tasks: usize,
    pub collect_cache_refs: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CpuPerfDelta {
    pub cycles: Option<u64>,
    pub instructions: Option<u64>,
    pub cache_references: Option<u64>,
    pub cache_misses: Option<u64>,

    pub ipc: Option<f64>,
    pub cache_miss_rate: Option<f64>,
    pub cache_mpki: Option<f64>,

    pub time_enabled_ns: Option<u64>,
    pub time_running_ns: Option<u64>,
    pub multiplexed: bool,
    pub scaled: bool,

    pub unavailable_reason: Option<String>,
}

pub struct CpuPerfSampler {
    config: CpuPerfConfig,
    groups: BTreeMap<u32, PerfCounterGroup>,
    skipped_tasks: BTreeSet<u32>,
    disabled_reason: Option<String>,
    last_error: Option<String>,
    total_read_errors: u64,
    total_open_errors: u64,
    total_samples: u64,
}

impl CpuPerfSampler {
    pub fn new(mut config: CpuPerfConfig) -> Self {
        if config.max_tasks == 0 {
            config.max_tasks = 1;
        }

        let event_count = event_count(config.collect_cache_refs) as u64;
        if let Some(available) = available_fd_budget() {
            let requested_fds = config.max_tasks as u64 * event_count;
            let half_budget = available / 2;
            if requested_fds > half_budget && half_budget >= event_count {
                let capped = (half_budget / event_count).max(1) as usize;
                warn!(
                    "cpu_perf requested up to {} tasks with {} events each ({} fds), but remaining fd budget is about {}; reducing cpu_perf_max_tasks to {}",
                    config.max_tasks, event_count, requested_fds, available, capped
                );
                config.max_tasks = config.max_tasks.min(capped);
            } else if requested_fds > available {
                warn!(
                    "cpu_perf requested up to {} tasks with {} events each ({} fds), but remaining fd budget is about {}; perf counters may be incomplete",
                    config.max_tasks, event_count, requested_fds, available
                );
            }
        }

        Self {
            config,
            groups: BTreeMap::new(),
            skipped_tasks: BTreeSet::new(),
            disabled_reason: None,
            last_error: None,
            total_read_errors: 0,
            total_open_errors: 0,
            total_samples: 0,
        }
    }

    pub fn sync_targets(
        &mut self,
        active_targets: &BTreeMap<u32, TaskInfo>,
        stats_by_task: &BTreeMap<u32, TaskStats>,
    ) {
        if self.disabled_reason.is_some() {
            self.groups.clear();
            return;
        }

        let selected = select_target_tids(active_targets, stats_by_task, self.config.max_tasks);
        self.skipped_tasks = active_targets
            .keys()
            .filter(|tid| !selected.contains(tid))
            .copied()
            .collect();

        self.groups.retain(|tid, _| selected.contains(tid));

        for tid in selected {
            if self.groups.contains_key(&tid) {
                continue;
            }

            match PerfCounterGroup::open(tid, &self.config) {
                Ok(group) => {
                    self.groups.insert(tid, group);
                }
                Err(err) => {
                    self.total_open_errors = self.total_open_errors.saturating_add(1);
                    let message = format!("task {}: {}", tid, err.message);
                    self.last_error = Some(message.clone());

                    if err.is_permission_denied() {
                        let reason = "cpu_perf unavailable: perf_event_open denied; check CAP_PERFMON/CAP_SYS_ADMIN or perf_event_paranoid".to_owned();
                        warn!("{reason}");
                        self.disabled_reason = Some(reason.clone());
                        self.last_error = Some(reason);
                        self.groups.clear();
                        break;
                    }

                    if !err.is_task_gone() {
                        warn!("cpu_perf_open_failed {message}");
                    }
                }
            }
        }
    }

    pub fn sample_interval(&mut self) -> BTreeMap<u32, CpuPerfDelta> {
        let mut deltas = BTreeMap::new();
        if let Some(reason) = &self.disabled_reason {
            self.last_error = Some(reason.clone());
            return deltas;
        }

        for (tid, group) in &mut self.groups {
            self.total_samples = self.total_samples.saturating_add(1);
            match group.sample_interval() {
                Ok(delta) => {
                    deltas.insert(*tid, delta);
                }
                Err(message) => {
                    self.total_read_errors = self.total_read_errors.saturating_add(1);
                    self.last_error = Some(format!("task {}: {}", tid, message));
                    deltas.insert(
                        *tid,
                        CpuPerfDelta {
                            unavailable_reason: Some(message),
                            ..Default::default()
                        },
                    );
                }
            }
        }

        deltas
    }

    pub fn active_counter_tasks(&self) -> usize {
        self.groups.len()
    }

    pub fn skipped_counter_tasks(&self) -> usize {
        self.skipped_tasks.len()
    }

    pub fn total_read_errors(&self) -> u64 {
        self.total_read_errors
    }

    pub fn total_open_errors(&self) -> u64 {
        self.total_open_errors
    }

    pub fn total_samples(&self) -> u64 {
        self.total_samples
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error
            .as_deref()
            .or(self.disabled_reason.as_deref())
    }
}

pub fn try_open_disabled_cycles_current_thread(include_kernel: bool) -> io::Result<()> {
    let tid = unsafe { libc::syscall(libc::SYS_gettid) as i32 };
    let mut attr = perf_event_attr(PERF_COUNT_HW_CPU_CYCLES, true, include_kernel);
    let fd = perf_event_open(&mut attr, tid, -1, -1)?;
    let _owned = OwnedFd::new(fd);
    Ok(())
}

#[repr(C)]
#[derive(Clone, Copy)]
struct PerfEventAttr {
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

struct PerfCounterGroup {
    leader: OwnedFd,
    siblings: Vec<OwnedFd>,
    events: Vec<PerfEventKind>,
}

impl PerfCounterGroup {
    fn open(tid: u32, config: &CpuPerfConfig) -> Result<Self, PerfOpenError> {
        let tid = tid as i32;
        let mut leader_attr =
            perf_event_attr(PERF_COUNT_HW_CPU_CYCLES, true, config.include_kernel);
        let leader_fd =
            perf_event_open(&mut leader_attr, tid, -1, -1).map_err(PerfOpenError::from_io)?;
        let leader = OwnedFd::new(leader_fd);

        let mut group = Self {
            leader,
            siblings: Vec::new(),
            events: vec![PerfEventKind::Cycles],
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
                warn!(
                    "cpu_perf_cache_references_unavailable tid={} err={}",
                    tid, err
                );
            } else {
                return Err(PerfOpenError::from_io(err));
            }
        }

        group
            .reset_and_enable()
            .map_err(|err| PerfOpenError::from_message(err.to_string()))?;
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

    fn sample_interval(&mut self) -> Result<CpuPerfDelta, String> {
        let expected = self.events.len();
        let mut values = [0u64; 7];
        let expected_bytes = (3 + expected) * mem::size_of::<u64>();
        let read_bytes = unsafe {
            libc::read(
                self.leader.fd,
                values.as_mut_ptr().cast::<libc::c_void>(),
                expected_bytes,
            )
        };
        if read_bytes < 0 {
            return Err(io::Error::last_os_error().to_string());
        }
        if read_bytes as usize != expected_bytes {
            return Err(format!(
                "short perf group read: got {} bytes, expected {}",
                read_bytes, expected_bytes
            ));
        }

        let nr = values[0] as usize;
        let time_enabled = values[1];
        let time_running = values[2];
        if nr != expected {
            return Ok(CpuPerfDelta {
                time_enabled_ns: Some(time_enabled),
                time_running_ns: Some(time_running),
                unavailable_reason: Some(format!(
                    "perf group returned {} values, expected {}",
                    nr, expected
                )),
                ..Default::default()
            });
        }

        let multiplexed = time_enabled != time_running && time_enabled > 0;
        let mut scaled = false;
        let mut delta = CpuPerfDelta {
            time_enabled_ns: Some(time_enabled),
            time_running_ns: Some(time_running),
            multiplexed,
            ..Default::default()
        };

        for (idx, kind) in self.events.iter().copied().enumerate() {
            let raw = values[3 + idx];
            let value = scale(raw, time_enabled, time_running);
            if multiplexed && value.is_some() {
                scaled = true;
            }
            match kind {
                PerfEventKind::Cycles => delta.cycles = value,
                PerfEventKind::Instructions => delta.instructions = value,
                PerfEventKind::CacheMisses => delta.cache_misses = value,
                PerfEventKind::CacheReferences => delta.cache_references = value,
            }
        }

        apply_derived_metrics(&mut delta);
        delta.scaled = scaled;

        if let Err(err) = self.reset_and_enable() {
            delta.unavailable_reason = Some(format!("perf group reset failed: {err}"));
        }

        Ok(delta)
    }

    fn reset_and_enable(&self) -> io::Result<()> {
        perf_ioctl(self.leader.fd, PERF_EVENT_IOC_DISABLE, PERF_IOC_FLAG_GROUP)?;
        perf_ioctl(self.leader.fd, PERF_EVENT_IOC_RESET, PERF_IOC_FLAG_GROUP)?;
        perf_ioctl(self.leader.fd, PERF_EVENT_IOC_ENABLE, PERF_IOC_FLAG_GROUP)?;
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum PerfEventKind {
    Cycles,
    Instructions,
    CacheMisses,
    CacheReferences,
}

struct OwnedFd {
    fd: RawFd,
}

impl OwnedFd {
    fn new(fd: RawFd) -> Self {
        Self { fd }
    }
}

impl Drop for OwnedFd {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.fd);
        }
    }
}

struct PerfOpenError {
    message: String,
    errno: Option<i32>,
}

impl PerfOpenError {
    fn from_io(err: io::Error) -> Self {
        Self {
            message: err.to_string(),
            errno: err.raw_os_error(),
        }
    }

    fn from_message(message: String) -> Self {
        Self {
            message,
            errno: None,
        }
    }

    fn is_permission_denied(&self) -> bool {
        matches!(self.errno, Some(libc::EACCES | libc::EPERM))
    }

    fn is_task_gone(&self) -> bool {
        matches!(self.errno, Some(libc::ESRCH))
    }
}

fn perf_event_attr(config: u64, disabled: bool, include_kernel: bool) -> PerfEventAttr {
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

fn perf_event_open(
    attr: &mut PerfEventAttr,
    pid: libc::pid_t,
    cpu: libc::c_int,
    group_fd: RawFd,
) -> io::Result<RawFd> {
    let fd = unsafe {
        libc::syscall(
            libc::SYS_perf_event_open,
            attr as *mut PerfEventAttr,
            pid,
            cpu,
            group_fd,
            PERF_FLAG_FD_CLOEXEC,
        )
    };
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(fd as RawFd)
    }
}

fn perf_ioctl(fd: RawFd, request: libc::c_ulong, arg: libc::c_ulong) -> io::Result<()> {
    let ret = unsafe { libc::ioctl(fd, request, arg) };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn event_count(collect_cache_refs: bool) -> usize {
    if collect_cache_refs { 4 } else { 3 }
}

fn select_target_tids(
    active_targets: &BTreeMap<u32, TaskInfo>,
    stats_by_task: &BTreeMap<u32, TaskStats>,
    max_tasks: usize,
) -> BTreeSet<u32> {
    let mut candidates = active_targets
        .iter()
        .map(|(tid, info)| (*tid, task_perf_priority(info, stats_by_task.get(tid))))
        .collect::<Vec<_>>();

    candidates.sort_by(|(left_tid, left_priority), (right_tid, right_priority)| {
        right_priority
            .cmp(left_priority)
            .then_with(|| left_tid.cmp(right_tid))
    });

    candidates
        .into_iter()
        .take(max_tasks)
        .map(|(tid, _)| tid)
        .collect()
}

fn task_perf_priority(info: &TaskInfo, stats: Option<&TaskStats>) -> u64 {
    let class_score = match info.class {
        TaskClass::Game => 1_000_000,
        TaskClass::GameScope => 900_000,
        TaskClass::Compositor => 800_000,
        TaskClass::Render => 750_000,
        TaskClass::WineServer => 700_000,
        TaskClass::GameHelper => 600_000,
        TaskClass::Launcher => 200_000,
        TaskClass::Service => 150_000,
        TaskClass::SteamRuntime => 100_000,
        TaskClass::Helper => 50_000,
        TaskClass::Unknown => 10_000,
        _ => 10_000,
    };

    let latency_score = stats
        .map(|stats| stats.session_latency.max_ns.min(1_000_000_000))
        .unwrap_or(0);

    class_score + latency_score
}

fn scale(raw: u64, time_enabled: u64, time_running: u64) -> Option<u64> {
    if time_running == 0 {
        return None;
    }
    if time_running == time_enabled {
        return Some(raw);
    }

    Some(((raw as u128 * time_enabled as u128) / time_running as u128) as u64)
}

fn apply_derived_metrics(delta: &mut CpuPerfDelta) {
    delta.ipc = ratio(delta.instructions, delta.cycles);
    delta.cache_miss_rate = ratio(delta.cache_misses, delta.cache_references);
    delta.cache_mpki = match (delta.cache_misses, delta.instructions) {
        (Some(misses), Some(instructions)) if instructions > 0 => {
            Some(misses as f64 * 1000.0 / instructions as f64).filter(|v| v.is_finite())
        }
        _ => None,
    };
}

fn ratio(numerator: Option<u64>, denominator: Option<u64>) -> Option<f64> {
    match (numerator, denominator) {
        (Some(numerator), Some(denominator)) if denominator > 0 => {
            Some(numerator as f64 / denominator as f64).filter(|v| v.is_finite())
        }
        _ => None,
    }
}

fn available_fd_budget() -> Option<u64> {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    let ret = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) };
    if ret != 0 || limit.rlim_cur == libc::RLIM_INFINITY {
        return None;
    }

    let open_count = fs::read_dir("/proc/self/fd").ok()?.count() as u64;
    Some(limit.rlim_cur.saturating_sub(open_count))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task_info(tid: u32, class: TaskClass) -> TaskInfo {
        TaskInfo {
            tid,
            process_pid: tid,
            process_ppid: 0,
            comm: format!("task-{tid}"),
            process_comm: format!("proc-{tid}"),
            process_starttime_ticks: Some(tid as u64),
            task_starttime_ticks: Some(tid as u64),
            exe_dev: None,
            exe_ino: None,
            class,
            sched_policy: None,
            from_cgroup: false,
        }
    }

    #[test]
    fn scales_perf_values() {
        assert_eq!(scale(100, 10, 10), Some(100));
        assert_eq!(scale(100, 20, 10), Some(200));
        assert_eq!(scale(100, 10, 0), None);
        assert_eq!(scale(u64::MAX, u64::MAX, u64::MAX), Some(u64::MAX));
    }

    #[test]
    fn derives_perf_ratios() {
        let mut delta = CpuPerfDelta {
            cycles: Some(100),
            instructions: Some(200),
            cache_references: Some(100),
            cache_misses: Some(10),
            ..Default::default()
        };

        apply_derived_metrics(&mut delta);

        assert_eq!(delta.ipc, Some(2.0));
        assert_eq!(delta.cache_miss_rate, Some(0.1));
        assert_eq!(delta.cache_mpki, Some(50.0));

        let mut zero = CpuPerfDelta {
            cycles: Some(0),
            instructions: Some(0),
            cache_references: Some(0),
            cache_misses: Some(10),
            ..Default::default()
        };
        apply_derived_metrics(&mut zero);
        assert_eq!(zero.ipc, None);
        assert_eq!(zero.cache_miss_rate, None);
        assert_eq!(zero.cache_mpki, None);
    }

    #[test]
    fn selects_high_priority_tasks_deterministically() {
        let active_targets = BTreeMap::from([
            (10, task_info(10, TaskClass::Helper)),
            (20, task_info(20, TaskClass::Game)),
            (30, task_info(30, TaskClass::Launcher)),
        ]);
        let selected = select_target_tids(&active_targets, &BTreeMap::new(), 2);

        assert_eq!(selected, BTreeSet::from([20, 30]));
    }

    #[test]
    fn latency_can_lift_spiky_game_helper() {
        let active_targets = BTreeMap::from([
            (10, task_info(10, TaskClass::GameHelper)),
            (20, task_info(20, TaskClass::Launcher)),
        ]);
        let mut stats = TaskStats::new(10, "helper".to_owned(), 0);
        stats.session_latency.max_ns = 900_000_000;
        let stats_by_task = BTreeMap::from([(10, stats)]);

        let selected = select_target_tids(&active_targets, &stats_by_task, 1);

        assert_eq!(selected, BTreeSet::from([10]));
    }

    #[test]
    fn cap_limits_selected_tasks_with_tid_tiebreaker() {
        let active_targets = BTreeMap::from([
            (30, task_info(30, TaskClass::Game)),
            (10, task_info(10, TaskClass::Game)),
            (20, task_info(20, TaskClass::Game)),
        ]);

        let selected = select_target_tids(&active_targets, &BTreeMap::new(), 2);

        assert_eq!(selected, BTreeSet::from([10, 20]));
    }
}
