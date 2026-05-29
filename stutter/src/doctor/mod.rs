mod capabilities;
mod ebpf;
pub mod ebpf_map_sizing;
mod hwmon;
mod irq;
mod kms;
mod mangohud;
pub mod model;
mod perf;
mod registry;
pub mod tracepoints;
mod utils;

use capabilities::daemon_capabilities_check;
use ebpf::{ebpf_build_check, ebpf_runtime_permission_check};
use ebpf_map_sizing::ebpf_map_sizing_check;
use hwmon::hwmon_check;
use irq::irq_selection_check;
#[cfg(test)]
pub(crate) use irq::suggested_gpu_irq_lines_from_text;
use kms::kms_timing_check;
pub use mangohud::check_mangohud_log_path;
pub use model::{DoctorCheck, DoctorInput, DoctorReport, DoctorStatus};
use perf::{cpu_perf_preflight_check, fault_probe_preflight_check};
use registry::probe_registry_check;
use tracepoints::tracepoint_check;

pub fn doctor_command(input: DoctorInput) -> anyhow::Result<()> {
    if input.tracepoint_dump {
        return tracepoints::tracepoint_dump_command(&input);
    }

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

#[cfg(test)]
mod tests;
