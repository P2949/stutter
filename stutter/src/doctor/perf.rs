use std::{collections::BTreeMap, fs, io, path::Path};
use super::model::{DoctorCheck, DoctorStatus};

pub(crate) fn fault_probe_preflight_check() -> DoctorCheck {
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

pub(crate) fn cpu_perf_preflight_check() -> DoctorCheck {
    cpu_perf_preflight_check_at(
        Path::new("/proc/sys/kernel/perf_event_paranoid"),
        Path::new("/sys/bus/event_source/devices/cpu/type"),
        || crate::perf_counters::try_open_disabled_cycles_current_thread(false),
    )
}

pub(crate) fn cpu_perf_preflight_check_at(
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
