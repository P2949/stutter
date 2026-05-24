use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

use serde::Serialize;

use crate::{
    daemon::capabilities::{CapabilityProbe, DaemonCapabilities},
    drm_tracepoints::{self, DrmTracepointFormat},
    ebpf_loader, hwmon,
    probe_catalog::ProbeStatus,
    probe_registry::PROBE_REGISTRY,
};

mod ebpf_map_sizing;

use ebpf_map_sizing::ebpf_map_sizing_check;

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
    pub kms_timing: bool,
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
        daemon_capabilities_check(),
        tracepoint_check(input),
        probe_registry_check(input),
    ];

    if input.faults {
        checks.push(fault_probe_preflight_check());
    }
    if input.cpu_perf {
        checks.push(cpu_perf_preflight_check());
    }
    if input.kms_timing {
        checks.push(kms_timing_check());
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

fn daemon_capabilities_check() -> DoctorCheck {
    let capabilities = CapabilityProbe::default().probe();
    daemon_capabilities_check_from_snapshot(capabilities)
}

fn daemon_capabilities_check_from_snapshot(capabilities: DaemonCapabilities) -> DoctorCheck {
    let unavailable = capabilities.unavailable_features();
    let mut details = BTreeMap::new();

    details.insert(
        "kernel_release".to_owned(),
        capabilities
            .kernel_release
            .as_deref()
            .unwrap_or("unknown")
            .to_owned(),
    );
    details.insert(
        "btf_available".to_owned(),
        yes_no(capabilities.btf_available),
    );
    details.insert(
        "sched_tracepoints_available".to_owned(),
        yes_no(capabilities.sched_tracepoints_available),
    );
    details.insert(
        "perf_permissions_likely".to_owned(),
        yes_no(capabilities.perf_permissions_likely),
    );
    details.insert(
        "perf_event_paranoid".to_owned(),
        capabilities
            .perf_event_paranoid
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_owned()),
    );
    details.insert(
        "cgroup_v2_available".to_owned(),
        yes_no(capabilities.cgroup_v2_available),
    );
    details.insert(
        "sched_ext_available".to_owned(),
        yes_no(capabilities.sched_ext_available),
    );
    details.insert(
        "uclamp_available".to_owned(),
        yes_no(capabilities.uclamp_available),
    );
    details.insert(
        "ionice_available".to_owned(),
        yes_no(capabilities.ionice_available),
    );
    details.insert(
        "irq_affinity_available".to_owned(),
        yes_no(capabilities.irq_affinity_available),
    );
    details.insert(
        "gpu_sysfs_available".to_owned(),
        yes_no(capabilities.gpu_sysfs_available),
    );

    let required_missing = !capabilities.btf_available || !capabilities.sched_tracepoints_available;
    let status = if required_missing {
        DoctorStatus::Warn
    } else {
        DoctorStatus::Pass
    };
    let message = if unavailable.is_empty() {
        "daemon capability probe found all known optional features".to_owned()
    } else {
        format!(
            "daemon capability probe missing or cannot confirm: {}",
            unavailable.join(", ")
        )
    };

    DoctorCheck {
        name: "daemon_capabilities".to_owned(),
        status,
        message,
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
            "kms_pageflip_timing" => input.kms_timing,
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
        report.block_io_correlation_basis.clone(),
    );
    if !report.block_io_correlation_basis.is_empty() {
        details.insert(
            "block_io_correlation_confidence".to_owned(),
            ebpf_loader::BlockIoCorrelationBasis::from_str(&report.block_io_correlation_basis)
                .confidence()
                .to_owned(),
        );
    }
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

fn kms_timing_check() -> DoctorCheck {
    let availability = drm_tracepoints::discover_kms_tracepoints_default();
    kms_timing_check_from_availability(availability)
}

fn kms_timing_check_from_availability(
    availability: drm_tracepoints::KmsTracepointAvailability,
) -> DoctorCheck {
    let mut details = BTreeMap::new();
    details.insert(
        "generic_drm_tracepoints".to_owned(),
        available_unavailable(!availability.generic_drm.is_empty()),
    );
    details.insert(
        "i915_pageflip_tracepoints".to_owned(),
        available_unavailable(!availability.i915.is_empty()),
    );
    details.insert(
        "amdgpu_pageflip_tracepoints".to_owned(),
        available_unavailable(!availability.amdgpu.is_empty()),
    );
    details.insert(
        "selected_provider".to_owned(),
        availability.selected_provider_name().to_owned(),
    );
    details.insert(
        "pageflip_request".to_owned(),
        format_tracepoint_ref(availability.pageflip_request.as_ref()),
    );
    details.insert(
        "pageflip_done".to_owned(),
        format_tracepoint_ref(availability.pageflip_done.as_ref()),
    );
    details.insert(
        "vblank_event".to_owned(),
        format_tracepoint_ref(availability.vblank_event.as_ref()),
    );
    details.insert(
        "atomic_commit".to_owned(),
        format_tracepoint_ref(availability.atomic_commit.as_ref()),
    );
    details.insert(
        "available_drm_tracepoints".to_owned(),
        format_tracepoint_names(&availability.generic_drm),
    );
    details.insert(
        "available_i915_tracepoints".to_owned(),
        format_tracepoint_names(&availability.i915),
    );
    details.insert(
        "available_amdgpu_tracepoints".to_owned(),
        format_tracepoint_names(&availability.amdgpu),
    );
    details.insert(
        "usable_crtc_id".to_owned(),
        yes_no(availability.has_usable_crtc_id()),
    );
    details.insert(
        "usable_sequence".to_owned(),
        yes_no(availability.has_usable_sequence()),
    );
    details.insert(
        "usable_timestamp".to_owned(),
        yes_no(availability.has_usable_timestamp()),
    );

    for (idx, warning) in availability.warnings.iter().enumerate() {
        details.insert(format!("warning_{idx}"), warning.clone());
    }

    let usable = availability.has_selected_tracepoints();
    DoctorCheck {
        name: "kms_timing".to_owned(),
        status: if usable {
            DoctorStatus::Pass
        } else {
            DoctorStatus::Warn
        },
        message: if usable {
            "KMS timing tracepoints are usable with medium confidence".to_owned()
        } else {
            "KMS timing unavailable: no supported pageflip/vblank tracepoints found".to_owned()
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

fn format_tracepoint_ref(format: Option<&DrmTracepointFormat>) -> String {
    format
        .map(|format| format!("{}/{}", format.category, format.name))
        .unwrap_or_else(|| "-".to_owned())
}

fn format_tracepoint_names(formats: &[DrmTracepointFormat]) -> String {
    if formats.is_empty() {
        "-".to_owned()
    } else {
        formats
            .iter()
            .map(|format| format.name.as_str())
            .collect::<Vec<_>>()
            .join(",")
    }
}

fn available_unavailable(value: bool) -> String {
    if value {
        "available".to_owned()
    } else {
        "unavailable".to_owned()
    }
}

fn yes_no(value: bool) -> String {
    if value {
        "yes".to_owned()
    } else {
        "no".to_owned()
    }
}
#[cfg(test)]
#[path = "doctor/tests.rs"]
mod tests;
