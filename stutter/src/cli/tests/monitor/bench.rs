//! Tests for bench CLI argument mapping into monitor configuration.
//!
//! Owns bench parsing regression tests. Does not own production monitor CLI validation or command
//! execution.

use std::{path::PathBuf, time::Duration};

use super::*;
use crate::commands::input::AppCommand;

fn parse_bench_command<const N: usize>(args: [&str; N]) -> anyhow::Result<AppCommand> {
    let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
    crate::cli::parse_app_command_from(args)
}

#[test]
fn parses_bench_baseline() {
    let command = parse_bench_command([
        "stutter",
        "bench",
        "--watch-process",
        "Game.exe",
        "--persistent",
        "--duration",
        "180",
        "--scenario",
        "route-a",
        "--role",
        "baseline",
    ])
    .unwrap();

    let AppCommand::Bench(input) = command else {
        panic!("expected bench command");
    };

    assert_eq!(input.role, "baseline");
    assert_eq!(input.run_name, "bench-baseline-route-a");
    assert_eq!(
        input.config.timing.max_duration,
        Some(Duration::from_secs(180))
    );
    assert_eq!(
        input.config.recording.run_name.as_deref(),
        Some("bench-baseline-route-a")
    );
    assert_eq!(
        input.config.target.watch_process.as_deref(),
        Some("Game.exe")
    );
    assert!(input.config.target.persistent);
}

#[test]
fn parses_bench_current() {
    let command = parse_bench_command([
        "stutter",
        "bench",
        "--watch-process",
        "Game.exe",
        "--duration",
        "120",
        "--scenario",
        "route-a",
        "--role",
        "current",
    ])
    .unwrap();

    let AppCommand::Bench(input) = command else {
        panic!("expected bench command");
    };

    assert_eq!(input.role, "current");
    assert_eq!(input.run_name, "bench-current-route-a");
    assert_eq!(
        input.config.recording.run_name.as_deref(),
        Some("bench-current-route-a")
    );
}

#[test]
fn rejects_zero_bench_duration() {
    let err = parse_bench_command([
        "stutter",
        "bench",
        "--duration",
        "0",
        "--scenario",
        "route-a",
    ])
    .unwrap_err();

    assert!(
        err.to_string()
            .contains("--duration must be greater than zero")
    );
}

#[test]
fn rejects_invalid_bench_role() {
    let err = parse_bench_command([
        "stutter",
        "bench",
        "--duration",
        "1",
        "--scenario",
        "route-a",
        "--role",
        "candidate",
    ])
    .unwrap_err();

    assert!(
        err.to_string()
            .contains("--role must be baseline or current")
    );
}

#[test]
fn rejects_empty_bench_scenario() {
    let err =
        parse_bench_command(["stutter", "bench", "--duration", "1", "--scenario", ""]).unwrap_err();

    assert!(err.to_string().contains("--scenario must not be empty"));
}

#[test]
fn bench_rejects_no_record_flag() {
    let err = parse_bench_command([
        "stutter",
        "bench",
        "--duration",
        "1",
        "--scenario",
        "route-a",
        "--no-record",
    ])
    .unwrap_err();

    assert!(
        err.to_string()
            .contains("bench --no-record is contradictory")
    );
}

#[test]
fn bench_preserves_focus_cli_presence_over_config_file_defaults() {
    let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "stutter-test-bench-preserves-focus-cli-presence-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let config_path = dir.join("config.toml");
    std::fs::write(
        &config_path,
        r#"
focus_source = "foreground"
foreground_source = "sway"
foreground_poll_ms = 777
foreground_max_stale_ms = 3000
"#,
    )
    .unwrap();

    struct EnvGuard {
        key: &'static str,
        old: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let old = std::env::var(key).ok();
            // SAFETY: callers hold TEST_MUTEX before mutating the process
            // environment, keeping these tests serialized.
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, old }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(old) = &self.old {
                // SAFETY: EnvGuard is dropped while TEST_MUTEX is held, so
                // restore mutations remain serialized.
                unsafe {
                    std::env::set_var(self.key, old);
                }
            } else {
                // SAFETY: EnvGuard is dropped while TEST_MUTEX is held, so
                // restore mutations remain serialized.
                unsafe {
                    std::env::remove_var(self.key);
                }
            }
        }
    }

    let _guard = EnvGuard::set("STUTTER_CONFIG", config_path.to_str().unwrap());

    let command = crate::cli::parse_app_command_from([
        "stutter",
        "bench",
        "--duration",
        "1",
        "--scenario",
        "cli-presence",
        "--focus-source",
        "heuristic",
        "--foreground-source",
        "auto",
        "--foreground-poll-ms",
        "1000",
        "--foreground-max-stale-ms",
        "2500",
    ])
    .unwrap();

    let AppCommand::Bench(input) = command else {
        panic!("expected bench command");
    };

    assert_eq!(input.config.focus.focus_source, FocusSource::Heuristic);
    assert_eq!(input.config.focus.foreground_source, ForegroundSource::Auto);
    assert_eq!(input.config.focus.foreground_poll_ms, 1000);
    assert_eq!(input.config.focus.foreground_max_stale_ms, 2500);
    assert!(!input.config.focus.foreground_window);

    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn bench_preserves_monitor_flags() {
    let command = parse_bench_command([
        "stutter",
        "bench",
        "--watch-process",
        "Game.exe",
        "--hwmon",
        "--mangohud-log",
        "/tmp/mango.csv",
        "--duration",
        "10",
        "--scenario",
        "route-a",
    ])
    .unwrap();

    let AppCommand::Bench(input) = command else {
        panic!("expected bench command");
    };

    assert_eq!(
        input.config.target.watch_process.as_deref(),
        Some("Game.exe")
    );
    assert!(input.config.probes.hwmon);
    assert_eq!(
        input.config.mangohud.log,
        Some(PathBuf::from("/tmp/mango.csv"))
    );
}
