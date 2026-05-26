use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonOverheadBudget {
    pub max_cpu_millis_per_second: u64,
    pub max_memory_bytes: u64,
    pub max_open_fds: usize,
    pub max_disk_write_bytes_per_minute: u64,
}

impl Default for DaemonOverheadBudget {
    fn default() -> Self {
        Self {
            max_cpu_millis_per_second: 50,
            max_memory_bytes: 128 * 1024 * 1024,
            max_open_fds: 256,
            max_disk_write_bytes_per_minute: 8 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonOverheadSnapshot {
    pub sample_duration_millis: u64,
    pub cpu_millis_per_second: u64,
    pub memory_rss_bytes: Option<u64>,
    pub open_fds: Option<usize>,
    pub disk_write_bytes_per_minute: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonOverheadIssue {
    pub reason_code: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonOverheadReport {
    pub budget: DaemonOverheadBudget,
    pub snapshot: DaemonOverheadSnapshot,
    pub within_budget: bool,
    pub issues: Vec<DaemonOverheadIssue>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DaemonOverheadMonitor {
    proc_root: PathBuf,
    budget: DaemonOverheadBudget,
}

impl Default for DaemonOverheadMonitor {
    fn default() -> Self {
        Self {
            proc_root: PathBuf::from("/proc/self"),
            budget: DaemonOverheadBudget::default(),
        }
    }
}

impl DaemonOverheadMonitor {
    pub fn new(proc_root: PathBuf, budget: DaemonOverheadBudget) -> Self {
        Self { proc_root, budget }
    }

    pub fn budget(&self) -> &DaemonOverheadBudget {
        &self.budget
    }

    pub fn sample_over_duration(&self, duration: Duration) -> DaemonOverheadReport {
        let before = ProcessResourceSample::read(&self.proc_root);
        let started = Instant::now();
        std::thread::sleep(duration);
        let elapsed = started.elapsed();
        let after = ProcessResourceSample::read(&self.proc_root);

        let snapshot = DaemonOverheadSnapshot {
            sample_duration_millis: duration_millis_u64(elapsed),
            cpu_millis_per_second: cpu_millis_per_second(&before, &after, elapsed),
            memory_rss_bytes: after.memory_rss_bytes,
            open_fds: count_open_fds(&self.proc_root),
            disk_write_bytes_per_minute: write_bytes_per_minute(&before, &after, elapsed),
        };

        evaluate_daemon_overhead(snapshot, self.budget.clone())
    }
}

pub fn evaluate_daemon_overhead(
    snapshot: DaemonOverheadSnapshot,
    budget: DaemonOverheadBudget,
) -> DaemonOverheadReport {
    let mut issues = Vec::new();

    if snapshot.cpu_millis_per_second > budget.max_cpu_millis_per_second {
        issues.push(DaemonOverheadIssue {
            reason_code: "cpu_overhead_high".to_owned(),
            message: format!(
                "CPU overhead {} ms/s exceeds budget {} ms/s",
                snapshot.cpu_millis_per_second, budget.max_cpu_millis_per_second
            ),
        });
    }

    if snapshot
        .memory_rss_bytes
        .is_some_and(|bytes| bytes > budget.max_memory_bytes)
    {
        issues.push(DaemonOverheadIssue {
            reason_code: "memory_overhead_high".to_owned(),
            message: format!(
                "RSS {} bytes exceeds budget {} bytes",
                snapshot.memory_rss_bytes.unwrap_or_default(),
                budget.max_memory_bytes
            ),
        });
    }

    if snapshot
        .open_fds
        .is_some_and(|fds| fds > budget.max_open_fds)
    {
        issues.push(DaemonOverheadIssue {
            reason_code: "fd_overhead_high".to_owned(),
            message: format!(
                "open fd count {} exceeds budget {}",
                snapshot.open_fds.unwrap_or_default(),
                budget.max_open_fds
            ),
        });
    }

    if snapshot
        .disk_write_bytes_per_minute
        .is_some_and(|bytes| bytes > budget.max_disk_write_bytes_per_minute)
    {
        issues.push(DaemonOverheadIssue {
            reason_code: "disk_write_overhead_high".to_owned(),
            message: format!(
                "disk writes {} bytes/min exceeds budget {} bytes/min",
                snapshot.disk_write_bytes_per_minute.unwrap_or_default(),
                budget.max_disk_write_bytes_per_minute
            ),
        });
    }

    DaemonOverheadReport {
        budget,
        snapshot,
        within_budget: issues.is_empty(),
        issues,
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ProcessResourceSample {
    cpu_jiffies: Option<u64>,
    memory_rss_bytes: Option<u64>,
    write_bytes: Option<u64>,
}

impl ProcessResourceSample {
    fn read(proc_root: &Path) -> Self {
        Self {
            cpu_jiffies: read_cpu_jiffies(&proc_root.join("stat")),
            memory_rss_bytes: read_status_rss_bytes(&proc_root.join("status")),
            write_bytes: read_proc_io_value(&proc_root.join("io"), "write_bytes:"),
        }
    }
}

fn cpu_millis_per_second(
    before: &ProcessResourceSample,
    after: &ProcessResourceSample,
    elapsed: Duration,
) -> u64 {
    let Some(before_jiffies) = before.cpu_jiffies else {
        return 0;
    };
    let Some(after_jiffies) = after.cpu_jiffies else {
        return 0;
    };
    let Some(delta_jiffies) = after_jiffies.checked_sub(before_jiffies) else {
        return 0;
    };
    let ticks_per_second = ticks_per_second().max(1);
    let cpu_millis = delta_jiffies.saturating_mul(1_000) / ticks_per_second;
    let elapsed_millis = duration_millis_u64(elapsed).max(1);
    cpu_millis.saturating_mul(1_000) / elapsed_millis
}

fn write_bytes_per_minute(
    before: &ProcessResourceSample,
    after: &ProcessResourceSample,
    elapsed: Duration,
) -> Option<u64> {
    let delta = after.write_bytes?.checked_sub(before.write_bytes?)?;
    let elapsed_millis = duration_millis_u64(elapsed).max(1);
    Some(delta.saturating_mul(60_000) / elapsed_millis)
}

fn duration_millis_u64(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

fn ticks_per_second() -> u64 {
    crate::syscall::clock_ticks_per_second()
}

fn read_cpu_jiffies(path: &Path) -> Option<u64> {
    let stat = fs::read_to_string(path).ok()?;
    let after_comm = stat.rsplit_once(") ")?.1;
    let fields: Vec<_> = after_comm.split_whitespace().collect();
    let utime = fields.get(11)?.parse::<u64>().ok()?;
    let stime = fields.get(12)?.parse::<u64>().ok()?;
    Some(utime.saturating_add(stime))
}

fn read_status_rss_bytes(path: &Path) -> Option<u64> {
    let status = fs::read_to_string(path).ok()?;
    status.lines().find_map(|line| {
        let value = line.strip_prefix("VmRSS:")?.split_whitespace().next()?;
        value
            .parse::<u64>()
            .ok()
            .map(|kib| kib.saturating_mul(1024))
    })
}

fn read_proc_io_value(path: &Path, key: &str) -> Option<u64> {
    let io = fs::read_to_string(path).ok()?;
    io.lines().find_map(|line| {
        let value = line.strip_prefix(key)?.trim();
        value.parse::<u64>().ok()
    })
}

fn count_open_fds(proc_root: &Path) -> Option<usize> {
    fs::read_dir(proc_root.join("fd"))
        .ok()
        .map(|entries| entries.count())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_proc(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-overhead-test-{name}-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        fs::create_dir_all(dir.join("fd")).unwrap();
        dir
    }

    #[test]
    fn overhead_evaluation_reports_all_budget_failures() {
        let report = evaluate_daemon_overhead(
            DaemonOverheadSnapshot {
                sample_duration_millis: 1_000,
                cpu_millis_per_second: 100,
                memory_rss_bytes: Some(2048),
                open_fds: Some(20),
                disk_write_bytes_per_minute: Some(4096),
            },
            DaemonOverheadBudget {
                max_cpu_millis_per_second: 10,
                max_memory_bytes: 1024,
                max_open_fds: 10,
                max_disk_write_bytes_per_minute: 1024,
            },
        );

        assert!(!report.within_budget);
        assert_eq!(report.issues.len(), 4);
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.reason_code == "cpu_overhead_high")
        );
    }

    #[test]
    fn overhead_evaluation_allows_missing_optional_metrics() {
        let report = evaluate_daemon_overhead(
            DaemonOverheadSnapshot {
                sample_duration_millis: 1_000,
                cpu_millis_per_second: 0,
                memory_rss_bytes: None,
                open_fds: None,
                disk_write_bytes_per_minute: None,
            },
            DaemonOverheadBudget::default(),
        );

        assert!(report.within_budget);
        assert!(report.issues.is_empty());
    }

    #[test]
    fn process_sample_reads_proc_like_files() {
        let proc_root = temp_proc("read");
        fs::write(
            proc_root.join("stat"),
            "123 (stutter test) S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15\n",
        )
        .unwrap();
        fs::write(proc_root.join("status"), "Name:\tstutter\nVmRSS:\t64 kB\n").unwrap();
        fs::write(proc_root.join("io"), "read_bytes: 1\nwrite_bytes: 4096\n").unwrap();

        let sample = ProcessResourceSample::read(&proc_root);

        assert_eq!(sample.cpu_jiffies, Some(23));
        assert_eq!(sample.memory_rss_bytes, Some(64 * 1024));
        assert_eq!(sample.write_bytes, Some(4096));
        assert_eq!(count_open_fds(&proc_root), Some(0));

        fs::remove_dir_all(proc_root).ok();
    }
}
