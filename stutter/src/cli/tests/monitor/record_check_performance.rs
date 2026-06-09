//! Tests for record/check/performance CLI compatibility with monitor arguments.
//!
//! Owns command parsing regression tests for adjacent CLI surfaces. Does not own production monitor
//! arguments or command execution.

use std::{path::PathBuf, time::Duration};

use super::*;
use crate::commands::input::AppCommand;

fn parse_cli_command<const N: usize>(args: [&str; N]) -> anyhow::Result<AppCommand> {
    let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
    crate::cli::parse_app_command_from(args)
}

#[test]
fn rejects_zero_duration_record() {
    let err =
        parse_cli_command(["stutter", "record", "--pid", "42", "--duration", "0"]).unwrap_err();

    assert!(
        err.to_string()
            .contains("--duration must be greater than zero")
    );
}

#[test]
fn record_command_forces_recording_mode_and_duration() {
    let config =
        parse_monitor_config_for_phase15(["stutter", "record", "--pid", "42", "--duration", "5"])
            .unwrap();

    assert_eq!(config.timing.max_duration, Some(Duration::from_secs(5)));
    assert_eq!(config.recording.run_name.as_deref(), Some("record"));
    assert!(config.probes.cpu_freq);
}

#[test]
fn record_rejects_no_record_flag() {
    let err = parse_cli_command(["stutter", "record", "--pid", "42", "--no-record"]).unwrap_err();

    assert!(
        err.to_string()
            .contains("record --no-record is contradictory")
    );
}

#[test]
fn parses_cpu_perf_monitor_flags() {
    let config = parse_monitor_config_for_phase15([
        "stutter",
        "monitor",
        "--pid",
        "42",
        "--cpu-perf",
        "--cpu-perf-kernel",
        "--cpu-perf-max-tasks",
        "16",
    ])
    .unwrap();

    assert!(config.probes.cpu_perf);
    assert!(config.cpu_perf.include_kernel);
    assert_eq!(config.cpu_perf.max_tasks, 16);
    assert!(!config.cpu_perf.collect_cache_refs);
}

#[test]
fn parses_cpu_perf_cache_refs_for_recording() {
    let config = parse_monitor_config_for_phase15([
        "stutter",
        "record",
        "--pid",
        "42",
        "--cpu-perf",
        "--cpu-perf-cache-refs",
    ])
    .unwrap();

    assert!(config.probes.cpu_perf);
    assert!(config.cpu_perf.collect_cache_refs);
    assert!(config.recording.output_dir.is_some() || config.recording.run_name.is_some());
}

#[test]
fn rejects_zero_cpu_perf_max_tasks() {
    let err = parse_monitor_config_for_phase15([
        "stutter",
        "monitor",
        "--pid",
        "42",
        "--cpu-perf-max-tasks",
        "0",
    ])
    .unwrap_err();

    assert!(
        err.to_string()
            .contains("--cpu-perf-max-tasks must be greater than zero")
    );
}

#[test]
fn parses_runtime_slices_monitor_flags() {
    let config = parse_monitor_config_for_phase15([
        "stutter",
        "monitor",
        "--pid",
        "42",
        "--runtime-slices",
        "--runtime-slices-max-tasks",
        "64",
    ])
    .unwrap();

    assert!(config.probes.runtime_slices);
    assert_eq!(config.runtime_slices.max_tasks, 64);
}

#[test]
fn rejects_zero_runtime_slices_max_tasks() {
    let err = parse_monitor_config_for_phase15([
        "stutter",
        "monitor",
        "--pid",
        "42",
        "--runtime-slices-max-tasks",
        "0",
    ])
    .unwrap_err();

    assert!(
        err.to_string()
            .contains("--runtime-slices-max-tasks must be greater than zero")
    );
}

#[test]
fn parses_ebpf_sizing_flags() {
    let config = parse_monitor_config_for_phase15([
        "stutter",
        "monitor",
        "--pid",
        "42",
        "--ebpf-ringbuf-size-kb",
        "1000",
        "--ebpf-wakeup-map-factor",
        "4",
        "--ebpf-block-start-entries",
        "32768",
        "--ebpf-drm-fence-wait-start-entries",
        "8192",
        "--ebpf-drm-fence-signal-entries",
        "8192",
    ])
    .unwrap();

    assert_eq!(config.ebpf_sizing.ringbuf_size_kb, Some(1000));
    assert_eq!(config.ebpf_sizing.wakeup_map_factor, Some(4));
    assert_eq!(config.ebpf_sizing.block_start_entries, Some(32768));
    assert_eq!(config.ebpf_sizing.drm_fence_wait_start_entries, Some(8192));
    assert_eq!(config.ebpf_sizing.drm_fence_signal_entries, Some(8192));
}

#[test]
fn parses_legacy_ebpf_sizing_aliases() {
    let config = parse_monitor_config_for_phase15([
        "stutter",
        "monitor",
        "--pid",
        "42",
        "--ringbuf-size-kb",
        "1000",
        "--wakeup-map-factor",
        "4",
    ])
    .unwrap();

    assert_eq!(config.ebpf_sizing.ringbuf_size_kb, Some(1000));
    assert_eq!(config.ebpf_sizing.wakeup_map_factor, Some(4));
}

#[test]
fn rejects_invalid_ringbuf_size_bounds() {
    for value in ["63", "16385"] {
        let err = parse_monitor_config_for_phase15([
            "stutter",
            "monitor",
            "--pid",
            "42",
            "--ebpf-ringbuf-size-kb",
            value,
        ])
        .unwrap_err();

        assert!(
            err.to_string()
                .contains("--ebpf-ringbuf-size-kb must be between 64 and 16384"),
            "expected ringbuf bound rejection for {value}, got {err:#}"
        );
    }
}

#[test]
fn rejects_invalid_wakeup_map_factor_bounds() {
    for value in ["0", "65"] {
        let err = parse_monitor_config_for_phase15([
            "stutter",
            "monitor",
            "--pid",
            "42",
            "--ebpf-wakeup-map-factor",
            value,
        ])
        .unwrap_err();

        assert!(
            err.to_string()
                .contains("--ebpf-wakeup-map-factor must be between 1 and 64"),
            "expected wakeup map factor rejection for {value}, got {err:#}"
        );
    }
}

#[test]
fn rejects_zero_ebpf_entry_cli_overrides() {
    for flag in [
        "--ebpf-block-start-entries",
        "--ebpf-drm-fence-wait-start-entries",
        "--ebpf-drm-fence-signal-entries",
    ] {
        let err =
            parse_monitor_config_for_phase15(["stutter", "monitor", "--pid", "42", flag, "0"])
                .unwrap_err();

        assert!(
            err.to_string().contains("must be greater than zero"),
            "expected zero rejection for {flag}, got {err:#}"
        );
    }
}

#[test]
fn parses_extended_check_command() {
    let command = parse_cli_command([
        "stutter",
        "check",
        "--baseline",
        "/tmp/base",
        "--current",
        "/tmp/current",
        "--max-regression-p99-ms",
        "0.5",
        "--max-max-regression-ms",
        "2.0",
        "--json",
        "--top",
        "5",
        "--filter-class",
        "Game",
    ])
    .unwrap();

    let AppCommand::Check(input) = command else {
        panic!("expected check command");
    };

    assert_eq!(input.baseline, PathBuf::from("/tmp/base"));
    assert_eq!(input.current, PathBuf::from("/tmp/current"));
    assert_eq!(input.max_regression_p99_ms, Some(0.5));
    assert_eq!(input.max_max_regression_ms, Some(2.0));
    assert!(input.json);
    assert_eq!(input.top, 5);
    assert_eq!(
        input.filter_class,
        Some(crate::process_tree::TaskClass::Game)
    );
}

#[test]
fn rejects_check_without_thresholds() {
    let err = parse_cli_command([
        "stutter",
        "check",
        "--baseline",
        "/tmp/base",
        "--current",
        "/tmp/current",
    ])
    .unwrap_err();

    assert!(
        err.to_string()
            .contains("check requires at least one threshold")
    );
}

#[test]
fn rejects_invalid_regression_threshold() {
    for value in ["NaN", "inf", "-1.0"] {
        let flag = format!("--max-regression-p99-ms={value}");
        let err = parse_cli_command([
            "stutter",
            "check",
            "--baseline",
            "run1/",
            "--current",
            "run2/",
            &flag,
        ])
        .unwrap_err();

        assert!(
            err.to_string()
                .contains("--max-regression-p99-ms must be a finite non-negative value"),
            "expected p99 threshold rejection for {value}, got {err:#}"
        );
    }
}

#[test]
fn rejects_zero_check_top() {
    let err = parse_cli_command([
        "stutter",
        "check",
        "--baseline",
        "/tmp/base",
        "--current",
        "/tmp/current",
        "--max-regression-p99-ms",
        "0.5",
        "--top",
        "0",
    ])
    .unwrap_err();

    assert!(err.to_string().contains("--top must be greater than zero"));
}

#[test]
fn parses_recording_retention_flags() {
    let config = parse_monitor_config_for_phase15([
        "stutter",
        "record",
        "--pid",
        "42",
        "--retention-max-runs",
        "7",
        "--retention-max-bytes",
        "2000000",
        "--retention-max-age-seconds",
        "86400",
        "--retention-min-free-bytes",
        "1000000",
    ])
    .unwrap();

    assert_eq!(config.recording.retention.max_run_count, Some(7));
    assert_eq!(config.recording.retention.max_total_bytes, Some(2_000_000));
    assert_eq!(config.recording.retention.max_age_seconds, Some(86_400));
    assert_eq!(config.recording.retention.min_free_bytes, Some(1_000_000));
}

#[test]
fn rejects_zero_recording_retention_flags() {
    for flag in [
        "--retention-max-runs",
        "--retention-max-bytes",
        "--retention-max-age-seconds",
        "--retention-min-free-bytes",
    ] {
        let err = parse_monitor_config_for_phase15(["stutter", "record", "--pid", "42", flag, "0"])
            .unwrap_err();

        assert!(
            err.to_string().contains("must be greater than zero"),
            "expected zero value rejection for {flag}, got {err:#}"
        );
    }
}

#[test]
fn parses_mangohud_log_flags_for_recording() {
    let config = parse_monitor_config_for_phase15([
        "stutter",
        "record",
        "--pid",
        "42",
        "--mangohud-log",
        "/tmp/mango.csv",
        "--mangohud-log-live",
    ])
    .unwrap();

    assert_eq!(config.mangohud.log, Some(PathBuf::from("/tmp/mango.csv")));
    assert!(config.mangohud.log_live);
}

#[test]
fn mangohud_log_live_requires_mangohud_log() {
    let err =
        parse_cli_command(["stutter", "record", "--pid", "42", "--mangohud-log-live"]).unwrap_err();

    assert!(
        err.to_string()
            .contains("required arguments were not provided")
            || err.to_string().contains("--mangohud-log")
    );
}
