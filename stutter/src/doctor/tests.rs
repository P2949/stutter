//! Tests extracted from the parent module to keep production files below the architecture size gate.

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
    let check =
        ebpf_runtime_permission_check_from_parts(0, Some((4096, 8192)), Ok(Some("2".to_owned())));
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
        kms_timing: false,
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
    assert!(
        report
            .checks
            .iter()
            .any(|c| c.name == "daemon_capabilities")
    );
}

#[test]
fn daemon_capability_check_reports_missing_required_features_as_warning() {
    let check = daemon_capabilities_check_from_snapshot(DaemonCapabilities {
        kernel_release: Some("6.9.1-test".to_owned()),
        btf_available: false,
        sched_tracepoints_available: true,
        perf_permissions_likely: true,
        perf_event_paranoid: Some(1),
        cgroup_v2_available: false,
        sched_ext_available: false,
        uclamp_available: false,
        ionice_available: true,
        irq_affinity_available: false,
        gpu_sysfs_available: false,
        privileged_worker_socket_reachable: Some(false),
    });

    assert_eq!(check.name, "daemon_capabilities");
    assert_eq!(check.status, DoctorStatus::Warn);
    assert_eq!(check.details["kernel_release"], "6.9.1-test");
    assert_eq!(check.details["btf_available"], "no");
    assert!(check.message.contains("btf"));
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
        kms_timing: false,
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
fn kms_timing_check_reports_usable_provider() {
    let availability = drm_tracepoints::KmsTracepointAvailability {
        pageflip_request: None,
        pageflip_done: None,
        vblank_event: Some(drm_tracepoints::parse_drm_tracepoint_format(
            "drm",
            "drm_vblank_event",
            "field:unsigned int crtc_id;\toffset:8;\tsize:4;\tsigned:0;\n\
                 field:unsigned int sequence;\toffset:12;\tsize:4;\tsigned:0;\n",
        )),
        atomic_commit: None,
        provider: drm_tracepoints::KmsTracepointProvider::GenericDrm,
        generic_drm: vec![drm_tracepoints::parse_drm_tracepoint_format(
            "drm",
            "drm_vblank_event",
            "field:unsigned int crtc_id;\toffset:8;\tsize:4;\tsigned:0;\n",
        )],
        i915: Vec::new(),
        amdgpu: Vec::new(),
        warnings: Vec::new(),
    };

    let check = kms_timing_check_from_availability(availability);

    assert_eq!(check.name, "kms_timing");
    assert_eq!(check.status, DoctorStatus::Pass);
    assert_eq!(check.details["selected_provider"], "generic_drm");
    assert_eq!(check.details["generic_drm_tracepoints"], "available");
    assert_eq!(check.details["usable_crtc_id"], "yes");
    assert_eq!(check.details["usable_timestamp"], "yes");
}

#[test]
fn ebpf_map_sizing_check_reports_target_and_wakeup_capacities() {
    let check = ebpf_map_sizing_check();

    assert_eq!(check.name, "ebpf_map_sizing");
    assert_eq!(
        check.details.get("target_pids_max"),
        Some(&crate::config::TARGET_PIDS_MAX.to_string())
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
