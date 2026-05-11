use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

use serde::Serialize;

use crate::{ebpf_loader, hwmon, probe_catalog::ProbeStatus, probe_registry::PROBE_REGISTRY};

#[derive(Debug, Clone)]
pub struct DoctorInput {
    pub json: bool,
    pub hwmon: bool,
    pub hwmon_root: Option<PathBuf>,
    pub hwmon_drm_card: Option<String>,
    pub hwmon_render_node: Option<PathBuf>,
    pub irq_latency: bool,
    pub irqs: Vec<u32>,
    pub block_io: bool,
    pub faults: bool,
    pub cpu_perf: bool,
    pub mangohud_log: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub enum DoctorStatus {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Serialize)]
pub struct DoctorReport {
    pub overall: DoctorStatus,
    pub checks: Vec<DoctorCheck>,
}

#[derive(Debug, Serialize)]
pub struct DoctorCheck {
    pub name: String,
    pub status: DoctorStatus,
    pub message: String,
    pub details: BTreeMap<String, String>,
}

pub fn doctor_command(input: DoctorInput) -> anyhow::Result<()> {
    let report = build_doctor_report(&input);

    if input.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_doctor_report(&report);
    }

    Ok(())
}

pub fn build_doctor_report(input: &DoctorInput) -> DoctorReport {
    let mut checks = vec![
        ebpf_build_check(),
        ebpf_runtime_permission_check(),
        ebpf_map_sizing_check(),
        tracepoint_check(input),
        probe_registry_check(input),
    ];

    if input.faults {
        checks.push(fault_probe_preflight_check());
    }
    if input.cpu_perf {
        checks.push(cpu_perf_preflight_check());
    }
    if input.hwmon {
        checks.push(hwmon_check(input));
    }
    if input.irq_latency {
        checks.push(irq_selection_check(&input.irqs));
    }
    if let Some(path) = &input.mangohud_log {
        checks.push(check_mangohud_log_path(path));
    }

    DoctorReport {
        overall: aggregate_status(&checks),
        checks,
    }
}

pub fn aggregate_status(checks: &[DoctorCheck]) -> DoctorStatus {
    if checks
        .iter()
        .any(|check| matches!(check.status, DoctorStatus::Fail))
    {
        DoctorStatus::Fail
    } else if checks
        .iter()
        .any(|check| matches!(check.status, DoctorStatus::Warn))
    {
        DoctorStatus::Warn
    } else {
        DoctorStatus::Pass
    }
}

fn print_doctor_report(report: &DoctorReport) {
    println!("stutter doctor");
    println!("==============");
    println!();
    println!("overall: {:?}", report.overall);
    println!();

    for check in &report.checks {
        println!("[{:?}] {}", check.status, check.name);
        println!("  {}", check.message);
        for (key, value) in &check.details {
            println!("  {key}={value}");
        }
        println!();
    }
}

fn ebpf_build_check() -> DoctorCheck {
    DoctorCheck {
        name: "ebpf_build".to_owned(),
        status: DoctorStatus::Pass,
        message: "binary started; eBPF object was embedded at build time".to_owned(),
        details: BTreeMap::new(),
    }
}

fn ebpf_runtime_permission_check() -> DoctorCheck {
    let euid = unsafe { libc::geteuid() };
    let rlimit = current_memlock_limit();
    let unprivileged_bpf_disabled =
        read_trimmed(Path::new("/proc/sys/kernel/unprivileged_bpf_disabled"));

    ebpf_runtime_permission_check_from_parts(euid, rlimit, unprivileged_bpf_disabled)
}

fn ebpf_runtime_permission_check_from_parts(
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

fn current_memlock_limit() -> Option<(u64, u64)> {
    let mut limit = std::mem::MaybeUninit::<libc::rlimit>::uninit();
    let rc = unsafe { libc::getrlimit(libc::RLIMIT_MEMLOCK, limit.as_mut_ptr()) };
    if rc != 0 {
        return None;
    }
    let limit = unsafe { limit.assume_init() };
    Some((limit.rlim_cur, limit.rlim_max))
}

fn format_rlimit_bytes(value: u64) -> String {
    if value == libc::RLIM_INFINITY {
        "unlimited".to_owned()
    } else {
        value.to_string()
    }
}

fn read_trimmed(path: &Path) -> Result<Option<String>, String> {
    match fs::read_to_string(path) {
        Ok(value) => Ok(Some(value.trim().to_owned())),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.to_string()),
    }
}

fn ebpf_map_sizing_check() -> DoctorCheck {
    let sizing = ebpf_loader::ebpf_map_sizing_report();
    let mut details = BTreeMap::new();
    details.insert(
        "locked_memory_limit_bytes".to_owned(),
        format_optional_u64(sizing.locked_memory_limit_bytes),
    );
    details.insert(
        "available_memory_bytes".to_owned(),
        format_optional_u64(sizing.available_memory_bytes),
    );
    details.insert(
        "events_ringbuf_bytes".to_owned(),
        sizing.events_ringbuf_bytes.to_string(),
    );
    details.insert(
        "target_pids_max".to_owned(),
        sizing.target_pids_max.to_string(),
    );
    details.insert(
        "wakeup_data_entries".to_owned(),
        sizing.wakeup_data_entries.to_string(),
    );

    let status = if sizing.events_ringbuf_bytes <= 64 * 1024 || sizing.wakeup_data_entries <= 4096 {
        DoctorStatus::Warn
    } else {
        DoctorStatus::Pass
    };

    DoctorCheck {
        name: "ebpf_map_sizing".to_owned(),
        status,
        message: if matches!(status, DoctorStatus::Pass) {
            "dynamic eBPF map sizing looks adequate".to_owned()
        } else {
            "dynamic eBPF map sizing is at the conservative minimum".to_owned()
        },
        details,
    }
}

fn probe_registry_check(input: &DoctorInput) -> DoctorCheck {
    let mut details = BTreeMap::new();
    let implemented = PROBE_REGISTRY
        .iter()
        .filter(|spec| spec.status == ProbeStatus::Implemented)
        .map(|spec| spec.catalog_key)
        .collect::<Vec<_>>()
        .join(",");

    let requested = PROBE_REGISTRY
        .iter()
        .filter(|spec| match spec.catalog_key {
            "scheduler_runnable_latency" => true,
            "cpu_freq" => true,
            "psi_timeline" => true,
            "irq_latency" => input.irq_latency,
            "gpu_hwmon" => input.hwmon,
            "block_io" => input.block_io,
            "faults" => input.faults,
            "cpu_perf" => input.cpu_perf,
            "frame_log" => input.mangohud_log.is_some(),
            _ => false,
        })
        .map(|spec| spec.catalog_key)
        .collect::<Vec<_>>()
        .join(",");

    details.insert("implemented_registry_probes".to_owned(), implemented);
    details.insert("requested_registry_probes".to_owned(), requested);

    DoctorCheck {
        name: "probe_registry".to_owned(),
        status: DoctorStatus::Pass,
        message: "probe metadata is loaded from PROBE_REGISTRY".to_owned(),
        details,
    }
}

fn tracepoint_check(input: &DoctorInput) -> DoctorCheck {
    let report = ebpf_loader::tracepoint_preflight(
        Path::new("/sys/kernel/tracing/events"),
        true,
        false,
        input.irq_latency,
        input.block_io,
        true,
    );

    let mut details = BTreeMap::new();
    details.insert("sched_wakeup".to_owned(), report.sched_wakeup);
    details.insert("sched_switch".to_owned(), report.sched_switch);
    details.insert("sched_wakeup_new".to_owned(), report.sched_wakeup_new);
    details.insert(
        "sched_wakeup_new_coverage".to_owned(),
        report.sched_wakeup_new_coverage,
    );
    details.insert("sched_migrate_task".to_owned(), report.sched_migrate_task);
    details.insert("cpu_frequency".to_owned(), report.cpu_frequency);
    details.insert("sched_stat_wait".to_owned(), report.sched_stat_wait);
    details.insert("irq_handler".to_owned(), report.irq_handler);
    details.insert("block_rq".to_owned(), report.block_rq);
    details.insert(
        "block_io_correlation_basis".to_owned(),
        report.block_io_correlation_basis,
    );
    for (idx, warning) in report.warnings.iter().enumerate() {
        details.insert(format!("warning_{idx}"), warning.clone());
    }
    for (idx, error) in report.errors.iter().enumerate() {
        details.insert(format!("error_{idx}"), error.clone());
    }

    let status = if !report.errors.is_empty() {
        DoctorStatus::Fail
    } else if !report.warnings.is_empty() {
        DoctorStatus::Warn
    } else {
        DoctorStatus::Pass
    };

    DoctorCheck {
        name: "tracepoint_formats".to_owned(),
        status,
        message: match status {
            DoctorStatus::Pass => "required tracepoint formats look compatible".to_owned(),
            DoctorStatus::Warn => {
                "required tracepoints look usable, but optional probes may be degraded".to_owned()
            }
            DoctorStatus::Fail => {
                "required scheduler tracepoint formats are missing or incompatible".to_owned()
            }
        },
        details,
    }
}

fn fault_probe_preflight_check() -> DoctorCheck {
    let mut details = BTreeMap::new();
    match fs::read_to_string("/proc/sys/kernel/perf_event_paranoid") {
        Ok(value) => {
            let trimmed = value.trim().to_owned();
            details.insert("perf_event_paranoid".to_owned(), trimmed.clone());
            let parsed = trimmed.parse::<i32>().ok();
            let status = if parsed.is_some_and(|value| value > 2) {
                DoctorStatus::Warn
            } else {
                DoctorStatus::Pass
            };
            DoctorCheck {
                name: "fault_probe_preflight".to_owned(),
                status,
                message: if matches!(status, DoctorStatus::Pass) {
                    "perf fault probes are not obviously blocked by perf_event_paranoid".to_owned()
                } else {
                    "perf_event_paranoid may restrict unprivileged fault probes".to_owned()
                },
                details,
            }
        }
        Err(err) => {
            details.insert("error".to_owned(), err.to_string());
            DoctorCheck {
                name: "fault_probe_preflight".to_owned(),
                status: DoctorStatus::Warn,
                message: "could not read perf_event_paranoid; fault probe support is uncertain"
                    .to_owned(),
                details,
            }
        }
    }
}

fn cpu_perf_preflight_check() -> DoctorCheck {
    cpu_perf_preflight_check_at(
        Path::new("/proc/sys/kernel/perf_event_paranoid"),
        Path::new("/sys/bus/event_source/devices/cpu/type"),
        || crate::perf_counters::try_open_disabled_cycles_current_thread(false),
    )
}

fn cpu_perf_preflight_check_at(
    perf_event_paranoid_path: &Path,
    cpu_pmu_type_path: &Path,
    opener: impl FnOnce() -> io::Result<()>,
) -> DoctorCheck {
    let mut details = BTreeMap::new();
    let mut warnings = Vec::new();

    match fs::read_to_string(perf_event_paranoid_path) {
        Ok(value) => {
            let trimmed = value.trim().to_owned();
            details.insert("perf_event_paranoid".to_owned(), trimmed.clone());
            if trimmed.parse::<i32>().is_ok_and(|value| value > 2) {
                warnings.push("perf_event_paranoid may restrict hardware counters".to_owned());
            }
        }
        Err(err) => {
            details.insert("perf_event_paranoid_error".to_owned(), err.to_string());
            warnings.push("could not read perf_event_paranoid".to_owned());
        }
    }

    match fs::read_to_string(cpu_pmu_type_path) {
        Ok(value) => {
            details.insert("cpu_pmu_type".to_owned(), value.trim().to_owned());
        }
        Err(err) => {
            details.insert("cpu_pmu_type_error".to_owned(), err.to_string());
            warnings.push("hardware PMU path missing".to_owned());
        }
    }

    match opener() {
        Ok(()) => {
            details.insert("cycles_open".to_owned(), "ok".to_owned());
            let status = if warnings.is_empty() {
                DoctorStatus::Pass
            } else {
                DoctorStatus::Warn
            };
            DoctorCheck {
                name: "cpu_perf_preflight".to_owned(),
                status,
                message: if matches!(status, DoctorStatus::Pass) {
                    "hardware perf counter opened successfully".to_owned()
                } else {
                    format!("hardware perf counter opened, but {}", warnings.join("; "))
                },
                details,
            }
        }
        Err(err) => {
            details.insert("cycles_open_error".to_owned(), err.to_string());
            let status = if matches!(err.raw_os_error(), Some(libc::EACCES | libc::EPERM)) {
                DoctorStatus::Fail
            } else {
                DoctorStatus::Warn
            };
            DoctorCheck {
                name: "cpu_perf_preflight".to_owned(),
                status,
                message: if matches!(status, DoctorStatus::Fail) {
                    "perf_event_open cycles failed with a permission error".to_owned()
                } else {
                    "hardware perf counter open failed; CPU perf telemetry may be unavailable"
                        .to_owned()
                },
                details,
            }
        }
    }
}

fn hwmon_check(input: &DoctorInput) -> DoctorCheck {
    let report = hwmon::probe_hwmon_with_options(
        input.hwmon_root.as_deref(),
        input.hwmon_drm_card.as_deref(),
        input.hwmon_render_node.as_deref(),
    );
    let mut details = BTreeMap::new();
    details.insert(
        "selected_root".to_owned(),
        report
            .selected_root
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "-".to_owned()),
    );
    details.insert(
        "gpu_busy_percent".to_owned(),
        yes_no(report.gpu_busy_available),
    );
    details.insert("vram_used".to_owned(), yes_no(report.vram_used_available));
    details.insert("vram_total".to_owned(), yes_no(report.vram_total_available));
    details.insert("temp".to_owned(), yes_no(report.temp_available));
    details.insert("power".to_owned(), yes_no(report.power_available));
    details.insert(
        "nvidia_smi_fallback".to_owned(),
        yes_no(report.nvidia_fallback_available),
    );
    for (idx, warning) in report.warnings.iter().enumerate() {
        details.insert(format!("warning_{idx}"), warning.clone());
    }

    let status = if report.warnings.is_empty()
        && (report.gpu_busy_available || report.nvidia_fallback_available)
    {
        DoctorStatus::Pass
    } else {
        DoctorStatus::Warn
    };

    DoctorCheck {
        name: "hwmon".to_owned(),
        status,
        message: if matches!(status, DoctorStatus::Pass) {
            "GPU hwmon telemetry appears available".to_owned()
        } else {
            "GPU hwmon telemetry may be missing or partial".to_owned()
        },
        details,
    }
}

fn irq_selection_check(irqs: &[u32]) -> DoctorCheck {
    let mut details = BTreeMap::new();
    if !irqs.is_empty() {
        details.insert(
            "irqs".to_owned(),
            irqs.iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(","),
        );
        return DoctorCheck {
            name: "irq_latency".to_owned(),
            status: DoctorStatus::Pass,
            message: "IRQ latency requested with explicit IRQ targets".to_owned(),
            details,
        };
    }

    let mut message =
        "no --irq supplied; inspect /proc/interrupts or use suggested GPU IRQ lines".to_owned();
    if let Ok(text) = fs::read_to_string("/proc/interrupts") {
        let suggestions = suggested_gpu_irq_lines_from_text(&text);
        for (idx, line) in suggestions.iter().take(8).enumerate() {
            details.insert(format!("suggested_irq_line_{idx}"), line.clone());
        }
        if suggestions.is_empty() {
            details.insert("suggestions".to_owned(), "none".to_owned());
        }
    } else {
        message.push_str("; /proc/interrupts was unreadable");
    }

    DoctorCheck {
        name: "irq_latency".to_owned(),
        status: DoctorStatus::Warn,
        message,
        details,
    }
}

pub fn suggested_gpu_irq_lines_from_text(text: &str) -> Vec<String> {
    const TERMS: &[&str] = &["amdgpu", "radeon", "nvidia", "i915", "xe", "drm", "gpu"];
    text.lines()
        .map(str::trim)
        .filter(|line| {
            let lower = line.to_ascii_lowercase();
            TERMS.iter().any(|term| lower.contains(term))
        })
        .map(str::to_owned)
        .collect()
}

pub fn check_mangohud_log_path(path: &Path) -> DoctorCheck {
    let mut details = BTreeMap::new();
    details.insert("path".to_owned(), path.display().to_string());

    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(err) => {
            details.insert("error".to_owned(), err.to_string());
            return DoctorCheck {
                name: "mangohud_log".to_owned(),
                status: DoctorStatus::Warn,
                message: "MangoHud log is missing or unreadable".to_owned(),
                details,
            };
        }
    };

    let mut buf = String::new();
    if let Err(err) = file.by_ref().take(8192).read_to_string(&mut buf) {
        details.insert("error".to_owned(), err.to_string());
        return DoctorCheck {
            name: "mangohud_log".to_owned(),
            status: DoctorStatus::Warn,
            message: "MangoHud log could not be read".to_owned(),
            details,
        };
    }

    if buf.trim().is_empty() {
        return DoctorCheck {
            name: "mangohud_log".to_owned(),
            status: DoctorStatus::Warn,
            message: "MangoHud log is empty".to_owned(),
            details,
        };
    }

    let looks_csv = buf.lines().any(|line| {
        let parts = line.split(',').map(str::trim).collect::<Vec<_>>();
        parts.len() >= 2 && parts.iter().take(2).all(|part| !part.is_empty())
    });

    DoctorCheck {
        name: "mangohud_log".to_owned(),
        status: if looks_csv {
            DoctorStatus::Pass
        } else {
            DoctorStatus::Warn
        },
        message: if looks_csv {
            "MangoHud log looks like comma-separated telemetry".to_owned()
        } else {
            "MangoHud log does not look like CSV telemetry".to_owned()
        },
        details,
    }
}

fn format_optional_u64(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unlimited_or_unknown".to_owned())
}

fn yes_no(value: bool) -> String {
    if value {
        "yes".to_owned()
    } else {
        "no".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(status: DoctorStatus) -> DoctorCheck {
        DoctorCheck {
            name: "test".to_owned(),
            status,
            message: String::new(),
            details: BTreeMap::new(),
        }
    }

    #[test]
    fn aggregate_status_prefers_fail_then_warn() {
        assert_eq!(
            aggregate_status(&[check(DoctorStatus::Pass), check(DoctorStatus::Fail)]),
            DoctorStatus::Fail
        );
        assert_eq!(
            aggregate_status(&[check(DoctorStatus::Pass), check(DoctorStatus::Warn)]),
            DoctorStatus::Warn
        );
        assert_eq!(
            aggregate_status(&[check(DoctorStatus::Pass)]),
            DoctorStatus::Pass
        );
    }

    #[test]
    fn suggested_gpu_irq_lines_match_known_driver_terms() {
        let text = "\
  45: 1 0 IO-APIC 45-fasteoi amdgpu
  46: 1 0 IO-APIC 46-fasteoi eth0
  47: 1 0 IO-APIC 47-fasteoi NVIDIA
";

        let lines = suggested_gpu_irq_lines_from_text(text);

        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("amdgpu"));
        assert!(lines[1].contains("NVIDIA"));
    }

    #[test]
    fn mangohud_log_checks_missing_empty_and_basic_csv() {
        let dir = temp_dir("doctor-mangohud");
        fs::create_dir_all(&dir).unwrap();

        let missing = check_mangohud_log_path(&dir.join("missing.csv"));
        assert_eq!(missing.status, DoctorStatus::Warn);

        let empty_path = dir.join("empty.csv");
        fs::write(&empty_path, "").unwrap();
        let empty = check_mangohud_log_path(&empty_path);
        assert_eq!(empty.status, DoctorStatus::Warn);

        let csv_path = dir.join("mangohud.csv");
        fs::write(&csv_path, "elapsed_ms,frametime_ms\n1,16.6\n").unwrap();
        let csv = check_mangohud_log_path(&csv_path);
        assert_eq!(csv.status, DoctorStatus::Pass);

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn cpu_perf_preflight_passes_when_cycles_open() {
        let dir = temp_dir("doctor-cpu-perf-pass");
        fs::create_dir_all(&dir).unwrap();
        let paranoid = dir.join("perf_event_paranoid");
        let pmu = dir.join("cpu_type");
        fs::write(&paranoid, "1\n").unwrap();
        fs::write(&pmu, "4\n").unwrap();

        let check = cpu_perf_preflight_check_at(&paranoid, &pmu, || Ok(()));

        assert_eq!(check.status, DoctorStatus::Pass);
        assert!(check.message.contains("opened successfully"));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn cpu_perf_preflight_fails_permission_denied() {
        let dir = temp_dir("doctor-cpu-perf-denied");
        fs::create_dir_all(&dir).unwrap();
        let paranoid = dir.join("perf_event_paranoid");
        let pmu = dir.join("cpu_type");
        fs::write(&paranoid, "4\n").unwrap();
        fs::write(&pmu, "4\n").unwrap();

        let check = cpu_perf_preflight_check_at(&paranoid, &pmu, || {
            Err(io::Error::from_raw_os_error(libc::EACCES))
        });

        assert_eq!(check.status, DoctorStatus::Fail);
        assert!(check.message.contains("permission"));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn ebpf_runtime_permission_check_passes_for_root() {
        let check = ebpf_runtime_permission_check_from_parts(
            0,
            Some((4096, 8192)),
            Ok(Some("2".to_owned())),
        );
        assert_eq!(check.name, "ebpf_runtime_permissions");
        assert_eq!(check.status, DoctorStatus::Pass);
        assert_eq!(check.details["effective_uid"], "0");
        assert_eq!(check.details["is_root"], "yes");
    }

    #[test]
    fn ebpf_runtime_permission_check_warns_for_non_root() {
        let check = ebpf_runtime_permission_check_from_parts(
            1000,
            Some((4096, 8192)),
            Ok(Some("2".to_owned())),
        );
        assert_eq!(check.status, DoctorStatus::Warn);
        assert!(check.message.contains("recording likely requires root"));
        assert!(check.message.contains("doas") || check.message.contains("sudo"));
        assert_eq!(check.details["effective_uid"], "1000");
        assert_eq!(check.details["is_root"], "no");
    }

    #[test]
    fn doctor_report_includes_runtime_permission_check() {
        let input = DoctorInput {
            json: false,
            hwmon: false,
            hwmon_root: None,
            hwmon_drm_card: None,
            hwmon_render_node: None,
            irq_latency: false,
            irqs: Vec::new(),
            block_io: false,
            faults: false,
            cpu_perf: false,
            mangohud_log: None,
        };

        let report = build_doctor_report(&input);
        assert!(
            report
                .checks
                .iter()
                .any(|c| c.name == "ebpf_runtime_permissions")
        );
    }

    #[test]
    fn doctor_tracepoint_check_reports_sched_wakeup_new_coverage() {
        let input = DoctorInput {
            json: false,
            hwmon: false,
            hwmon_root: None,
            hwmon_drm_card: None,
            hwmon_render_node: None,
            irq_latency: false,
            irqs: Vec::new(),
            block_io: false,
            faults: false,
            cpu_perf: false,
            mangohud_log: None,
        };

        let report = build_doctor_report(&input);
        let tracepoint_check = report
            .checks
            .iter()
            .find(|check| check.name == "tracepoint_formats")
            .expect("tracepoint_formats check should be present");

        assert!(tracepoint_check.details.contains_key("sched_wakeup_new"));
        assert!(
            tracepoint_check
                .details
                .contains_key("sched_wakeup_new_coverage")
        );
    }

    #[test]
    fn ebpf_map_sizing_check_reports_target_and_wakeup_capacities() {
        let check = ebpf_map_sizing_check();

        assert_eq!(check.name, "ebpf_map_sizing");
        assert_eq!(
            check.details.get("target_pids_max"),
            Some(&crate::cli::TARGET_PIDS_MAX.to_string())
        );
        assert!(check.details.contains_key("wakeup_data_entries"));
    }

    #[test]
    fn ebpf_runtime_permission_check_handles_missing_unprivileged_bpf_file() {
        let check = ebpf_runtime_permission_check_from_parts(1000, Some((4096, 8192)), Ok(None));
        assert_eq!(check.details["unprivileged_bpf_disabled"], "missing");
    }

    #[test]
    fn format_rlimit_bytes_marks_infinity_as_unlimited() {
        assert_eq!(format_rlimit_bytes(libc::RLIM_INFINITY), "unlimited");
        assert_eq!(format_rlimit_bytes(4096), "4096");
    }

    fn temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        dir
    }
}
