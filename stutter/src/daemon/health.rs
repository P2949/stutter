use std::{
    ffi::CString,
    io,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SystemHealthState {
    Healthy,
    Degraded,
    Overheated,
    LowDisk,
    LowMemory,
    InstrumentationBroken,
    SuspendedResumed,
}

impl SystemHealthState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Degraded => "degraded",
            Self::Overheated => "overheated",
            Self::LowDisk => "low_disk",
            Self::LowMemory => "low_memory",
            Self::InstrumentationBroken => "instrumentation_broken",
            Self::SuspendedResumed => "suspended_resumed",
        }
    }

    fn severity_rank(self) -> u8 {
        match self {
            Self::Healthy => 0,
            Self::Degraded => 1,
            Self::SuspendedResumed => 2,
            Self::InstrumentationBroken => 3,
            Self::LowMemory => 4,
            Self::LowDisk => 5,
            Self::Overheated => 6,
        }
    }

    pub fn blocks_apply(self) -> bool {
        !matches!(self, Self::Healthy)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SystemHealthInputs {
    pub max_cpu_temp_millidegrees: Option<i64>,
    pub max_gpu_temp_millidegrees: Option<i64>,
    pub ac_online: Option<bool>,
    pub battery_present: bool,
    pub memory_pressure_some_avg10_millipercent: Option<u32>,
    pub load_average_1m_milli: Option<u32>,
    pub cpu_count: Option<usize>,
    pub disk_available_bytes: Option<u64>,
    pub ebpf_dropped_events: u64,
    pub suspended_or_resumed: bool,
    pub probe_errors: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SystemHealthIssue {
    pub state: SystemHealthState,
    pub reason_code: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SystemHealthSnapshot {
    pub state: SystemHealthState,
    pub ok_for_apply: bool,
    pub reason_code: Option<String>,
    pub unix_nanos: Option<u128>,
    pub inputs: SystemHealthInputs,
    pub issues: Vec<SystemHealthIssue>,
}

impl Default for SystemHealthSnapshot {
    fn default() -> Self {
        Self {
            state: SystemHealthState::Healthy,
            ok_for_apply: true,
            reason_code: None,
            unix_nanos: None,
            inputs: SystemHealthInputs::default(),
            issues: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SystemHealthThresholds {
    pub max_cpu_temp_millidegrees: i64,
    pub max_gpu_temp_millidegrees: i64,
    pub min_disk_available_bytes: u64,
    pub max_memory_pressure_some_avg10_millipercent: u32,
    pub max_load_per_cpu_milli: u32,
    pub max_ebpf_dropped_events: u64,
}

impl Default for SystemHealthThresholds {
    fn default() -> Self {
        Self {
            max_cpu_temp_millidegrees: 90_000,
            max_gpu_temp_millidegrees: 92_000,
            min_disk_available_bytes: 512 * 1024 * 1024,
            max_memory_pressure_some_avg10_millipercent: 50_000,
            max_load_per_cpu_milli: 4_000,
            max_ebpf_dropped_events: 10_000,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemHealthProbeRoot {
    pub proc_root: PathBuf,
    pub sys_root: PathBuf,
    pub disk_path: PathBuf,
}

impl Default for SystemHealthProbeRoot {
    fn default() -> Self {
        Self {
            proc_root: PathBuf::from("/proc"),
            sys_root: PathBuf::from("/sys"),
            disk_path: PathBuf::from("."),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SystemHealthMonitor {
    root: SystemHealthProbeRoot,
    thresholds: SystemHealthThresholds,
}

impl SystemHealthMonitor {
    pub fn new(root: SystemHealthProbeRoot, thresholds: SystemHealthThresholds) -> Self {
        Self { root, thresholds }
    }

    pub fn thresholds(&self) -> &SystemHealthThresholds {
        &self.thresholds
    }

    pub fn evaluate(&self, inputs: SystemHealthInputs) -> SystemHealthSnapshot {
        evaluate_system_health(inputs, &self.thresholds)
    }

    pub fn probe(&self) -> SystemHealthSnapshot {
        self.evaluate(self.probe_inputs())
    }

    pub fn probe_inputs(&self) -> SystemHealthInputs {
        let mut inputs = SystemHealthInputs::default();

        self.read_temperatures(&mut inputs);
        self.read_power_supply(&mut inputs);
        self.read_proc_pressure(&mut inputs);
        self.read_load_average(&mut inputs);
        self.read_cpu_count(&mut inputs);
        self.read_disk_available(&mut inputs);

        inputs
    }

    fn read_temperatures(&self, inputs: &mut SystemHealthInputs) {
        let thermal_root = self.root.sys_root.join("class/thermal");
        read_thermal_zone_temperatures(&thermal_root, inputs);

        let hwmon_root = self.root.sys_root.join("class/hwmon");
        read_hwmon_temperatures(&hwmon_root, inputs);
    }

    fn read_power_supply(&self, inputs: &mut SystemHealthInputs) {
        let power_root = self.root.sys_root.join("class/power_supply");
        let Ok(entries) = std::fs::read_dir(&power_root) else {
            return;
        };

        let mut ac_online = false;
        let mut saw_ac = false;

        for entry in entries.flatten() {
            let path = entry.path();
            let supply_type = read_trimmed(&path.join("type"))
                .unwrap_or_default()
                .to_ascii_lowercase();
            match supply_type.as_str() {
                "battery" => inputs.battery_present = true,
                "mains" | "usb" | "usb_c" | "usb-c" | "ac" => {
                    saw_ac = true;
                    ac_online |= read_trimmed(&path.join("online"))
                        .as_deref()
                        .is_some_and(|value| value == "1");
                }
                _ => {}
            }
        }

        if saw_ac {
            inputs.ac_online = Some(ac_online);
        }
    }

    fn read_proc_pressure(&self, inputs: &mut SystemHealthInputs) {
        let path = self.root.proc_root.join("pressure/memory");
        let Some(text) = read_trimmed(&path) else {
            return;
        };
        inputs.memory_pressure_some_avg10_millipercent = parse_pressure_avg10(&text);
    }

    fn read_load_average(&self, inputs: &mut SystemHealthInputs) {
        let Some(text) = read_trimmed(&self.root.proc_root.join("loadavg")) else {
            return;
        };
        inputs.load_average_1m_milli = text.split_whitespace().next().and_then(parse_decimal_milli);
    }

    fn read_cpu_count(&self, inputs: &mut SystemHealthInputs) {
        let Some(text) = read_trimmed(&self.root.proc_root.join("stat")) else {
            return;
        };
        let count = text
            .lines()
            .filter(|line| {
                line.strip_prefix("cpu")
                    .and_then(|rest| rest.chars().next())
                    .is_some_and(|ch| ch.is_ascii_digit())
            })
            .count();
        if count > 0 {
            inputs.cpu_count = Some(count);
        }
    }

    fn read_disk_available(&self, inputs: &mut SystemHealthInputs) {
        match available_bytes_for_path(&self.root.disk_path) {
            Ok(bytes) => inputs.disk_available_bytes = Some(bytes),
            Err(err) => inputs
                .probe_errors
                .push(format!("disk_available_probe_failed: {err}")),
        }
    }
}

pub fn evaluate_system_health(
    inputs: SystemHealthInputs,
    thresholds: &SystemHealthThresholds,
) -> SystemHealthSnapshot {
    let mut issues = Vec::new();

    if !inputs.probe_errors.is_empty() {
        issues.push(SystemHealthIssue {
            state: SystemHealthState::InstrumentationBroken,
            reason_code: "instrumentation_probe_failed".to_owned(),
            message: format!(
                "system health probe reported {} error(s)",
                inputs.probe_errors.len()
            ),
        });
    }

    if inputs.suspended_or_resumed {
        issues.push(SystemHealthIssue {
            state: SystemHealthState::SuspendedResumed,
            reason_code: "suspend_resume_stabilizing".to_owned(),
            message: "system recently suspended or resumed".to_owned(),
        });
    }

    if inputs
        .max_cpu_temp_millidegrees
        .is_some_and(|value| value >= thresholds.max_cpu_temp_millidegrees)
    {
        issues.push(SystemHealthIssue {
            state: SystemHealthState::Overheated,
            reason_code: "cpu_overheated".to_owned(),
            message: "CPU temperature exceeds daemon guardrail".to_owned(),
        });
    }

    if inputs
        .max_gpu_temp_millidegrees
        .is_some_and(|value| value >= thresholds.max_gpu_temp_millidegrees)
    {
        issues.push(SystemHealthIssue {
            state: SystemHealthState::Overheated,
            reason_code: "gpu_overheated".to_owned(),
            message: "GPU temperature exceeds daemon guardrail".to_owned(),
        });
    }

    if inputs
        .disk_available_bytes
        .is_some_and(|value| value < thresholds.min_disk_available_bytes)
    {
        issues.push(SystemHealthIssue {
            state: SystemHealthState::LowDisk,
            reason_code: "low_disk".to_owned(),
            message: "free disk space is below daemon guardrail".to_owned(),
        });
    }

    if inputs
        .memory_pressure_some_avg10_millipercent
        .is_some_and(|value| value >= thresholds.max_memory_pressure_some_avg10_millipercent)
    {
        issues.push(SystemHealthIssue {
            state: SystemHealthState::LowMemory,
            reason_code: "memory_pressure_high".to_owned(),
            message: "memory PSI some avg10 exceeds daemon guardrail".to_owned(),
        });
    }

    if let (Some(load), Some(cpus)) = (inputs.load_average_1m_milli, inputs.cpu_count) {
        let per_cpu = load / cpus.max(1) as u32;
        if per_cpu >= thresholds.max_load_per_cpu_milli {
            issues.push(SystemHealthIssue {
                state: SystemHealthState::Degraded,
                reason_code: "load_average_high".to_owned(),
                message: "load average per CPU exceeds daemon guardrail".to_owned(),
            });
        }
    }

    if inputs.ebpf_dropped_events > thresholds.max_ebpf_dropped_events {
        issues.push(SystemHealthIssue {
            state: SystemHealthState::InstrumentationBroken,
            reason_code: "drop_counters_high".to_owned(),
            message: "eBPF dropped event counters exceed daemon guardrail".to_owned(),
        });
    }

    issues.sort_by_key(|issue| std::cmp::Reverse(issue.state.severity_rank()));
    let state = issues
        .first()
        .map(|issue| issue.state)
        .unwrap_or(SystemHealthState::Healthy);
    let reason_code = issues.first().map(|issue| issue.reason_code.clone());

    SystemHealthSnapshot {
        state,
        ok_for_apply: !state.blocks_apply(),
        reason_code,
        unix_nanos: Some(crate::audit::unix_nanos_now()),
        inputs,
        issues,
    }
}

fn read_thermal_zone_temperatures(root: &Path, inputs: &mut SystemHealthInputs) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(temp) = read_trimmed(&path.join("temp")).and_then(|value| value.parse().ok())
        else {
            continue;
        };
        let label = read_trimmed(&path.join("type"))
            .unwrap_or_default()
            .to_ascii_lowercase();
        if label.contains("gpu") {
            update_max(&mut inputs.max_gpu_temp_millidegrees, temp);
        } else {
            update_max(&mut inputs.max_cpu_temp_millidegrees, temp);
        }
    }
}

fn read_hwmon_temperatures(root: &Path, inputs: &mut SystemHealthInputs) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let chip_name = read_trimmed(&path.join("name"))
            .unwrap_or_default()
            .to_ascii_lowercase();
        let Ok(files) = std::fs::read_dir(&path) else {
            continue;
        };
        for file in files.flatten() {
            let path = file.path();
            let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !file_name.starts_with("temp") || !file_name.ends_with("_input") {
                continue;
            }
            let Some(temp) = read_trimmed(&path).and_then(|value| value.parse().ok()) else {
                continue;
            };
            if chip_name.contains("gpu")
                || chip_name.contains("amdgpu")
                || chip_name.contains("nvidia")
            {
                update_max(&mut inputs.max_gpu_temp_millidegrees, temp);
            } else {
                update_max(&mut inputs.max_cpu_temp_millidegrees, temp);
            }
        }
    }
}

fn update_max(slot: &mut Option<i64>, value: i64) {
    *slot = Some(slot.map_or(value, |current| current.max(value)));
}

fn parse_pressure_avg10(text: &str) -> Option<u32> {
    text.lines()
        .find(|line| line.starts_with("some "))
        .and_then(|line| {
            line.split_whitespace()
                .find_map(|part| part.strip_prefix("avg10=").and_then(parse_decimal_milli))
        })
}

fn parse_decimal_milli(value: &str) -> Option<u32> {
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    let whole = whole.parse::<u32>().ok()?;
    let mut fraction_digits = fraction.chars().take(3).collect::<String>();
    while fraction_digits.len() < 3 {
        fraction_digits.push('0');
    }
    let fraction = if fraction_digits.is_empty() {
        0
    } else {
        fraction_digits.parse::<u32>().ok()?
    };
    Some(whole.saturating_mul(1_000).saturating_add(fraction))
}

fn read_trimmed(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn available_bytes_for_path(path: &Path) -> io::Result<u64> {
    let existing_path = nearest_existing_path(path);
    let c_path = CString::new(existing_path.as_os_str().as_bytes())
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    let rc = unsafe { libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr()) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    let stat = unsafe { stat.assume_init() };
    Ok(stat.f_bavail.saturating_mul(stat.f_frsize))
}

fn nearest_existing_path(path: &Path) -> PathBuf {
    let mut current = path;
    loop {
        if current.exists() {
            return current.to_path_buf();
        }
        let Some(parent) = current.parent() else {
            return PathBuf::from(".");
        };
        current = parent;
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-system-health-test-{name}-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_file(path: &Path, value: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, value).unwrap();
    }

    #[test]
    fn healthy_inputs_do_not_block_apply() {
        let monitor = SystemHealthMonitor::default();

        let snapshot = monitor.evaluate(SystemHealthInputs {
            max_cpu_temp_millidegrees: Some(55_000),
            max_gpu_temp_millidegrees: Some(60_000),
            disk_available_bytes: Some(10 * 1024 * 1024 * 1024),
            memory_pressure_some_avg10_millipercent: Some(500),
            load_average_1m_milli: Some(2_000),
            cpu_count: Some(8),
            ..SystemHealthInputs::default()
        });

        assert_eq!(snapshot.state, SystemHealthState::Healthy);
        assert!(snapshot.ok_for_apply);
        assert!(snapshot.reason_code.is_none());
    }

    #[test]
    fn overheating_wins_over_lower_severity_issues() {
        let monitor = SystemHealthMonitor::default();

        let snapshot = monitor.evaluate(SystemHealthInputs {
            max_cpu_temp_millidegrees: Some(95_000),
            disk_available_bytes: Some(1),
            ..SystemHealthInputs::default()
        });

        assert_eq!(snapshot.state, SystemHealthState::Overheated);
        assert!(!snapshot.ok_for_apply);
        assert_eq!(snapshot.reason_code.as_deref(), Some("cpu_overheated"));
        assert!(
            snapshot
                .issues
                .iter()
                .any(|issue| issue.reason_code == "low_disk")
        );
    }

    #[test]
    fn high_memory_pressure_and_drops_block_apply() {
        let monitor = SystemHealthMonitor::default();

        let snapshot = monitor.evaluate(SystemHealthInputs {
            memory_pressure_some_avg10_millipercent: Some(80_000),
            ebpf_dropped_events: 20_000,
            ..SystemHealthInputs::default()
        });

        assert!(!snapshot.ok_for_apply);
        assert!(
            snapshot
                .issues
                .iter()
                .any(|issue| issue.reason_code == "memory_pressure_high")
        );
        assert!(
            snapshot
                .issues
                .iter()
                .any(|issue| issue.reason_code == "drop_counters_high")
        );
    }

    #[test]
    fn probe_reads_fake_proc_sys_inputs() {
        let root = temp_root("probe");
        let proc_root = root.join("proc");
        let sys_root = root.join("sys");

        write_file(
            &proc_root.join("pressure/memory"),
            "some avg10=12.50 avg60=0.00 avg300=0.00 total=1\n",
        );
        write_file(&proc_root.join("loadavg"), "4.50 1.00 0.50 1/10 99\n");
        write_file(
            &proc_root.join("stat"),
            "cpu  1 2 3\ncpu0 1 2 3\ncpu1 1 2 3\n",
        );
        write_file(
            &sys_root.join("class/thermal/thermal_zone0/type"),
            "x86_pkg_temp\n",
        );
        write_file(
            &sys_root.join("class/thermal/thermal_zone0/temp"),
            "65000\n",
        );
        write_file(&sys_root.join("class/hwmon/hwmon0/name"), "amdgpu\n");
        write_file(&sys_root.join("class/hwmon/hwmon0/temp1_input"), "70000\n");
        write_file(&sys_root.join("class/power_supply/AC/type"), "Mains\n");
        write_file(&sys_root.join("class/power_supply/AC/online"), "1\n");
        write_file(&sys_root.join("class/power_supply/BAT0/type"), "Battery\n");

        let monitor = SystemHealthMonitor::new(
            SystemHealthProbeRoot {
                proc_root,
                sys_root,
                disk_path: root.clone(),
            },
            SystemHealthThresholds::default(),
        );

        let snapshot = monitor.probe();

        assert_eq!(snapshot.inputs.max_cpu_temp_millidegrees, Some(65_000));
        assert_eq!(snapshot.inputs.max_gpu_temp_millidegrees, Some(70_000));
        assert_eq!(
            snapshot.inputs.memory_pressure_some_avg10_millipercent,
            Some(12_500)
        );
        assert_eq!(snapshot.inputs.load_average_1m_milli, Some(4_500));
        assert_eq!(snapshot.inputs.cpu_count, Some(2));
        assert_eq!(snapshot.inputs.ac_online, Some(true));
        assert!(snapshot.inputs.battery_present);

        fs::remove_dir_all(root).ok();
    }
}
