use std::path::PathBuf;

use crate::{
    commands::input::AppCommand,
    service::{ServiceAction, ServiceManager, ServiceMode},
};

fn parse_service_command<const N: usize>(args: [&str; N]) -> anyhow::Result<AppCommand> {
    let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
    crate::cli::parse_app_command_from(args)
}

#[test]
fn parses_service_install_dry_run_command() {
    let command = parse_service_command([
        "stutter",
        "service",
        "install",
        "--mode",
        "system-low-risk",
        "--manager",
        "openrc",
        "--unit-dir",
        "/tmp/init.d",
        "--config-dir",
        "/tmp/stutter-config",
        "--state-dir",
        "/tmp/stutter-state",
        "--log-dir",
        "/tmp/stutter-log",
        "--dry-run",
        "--json",
    ])
    .unwrap();

    let AppCommand::Service(input) = command else {
        panic!("expected service command");
    };

    assert!(input.json);
    assert_eq!(input.request.action, ServiceAction::Install);
    assert_eq!(input.request.manager, ServiceManager::OpenRc);
    assert_eq!(input.request.mode, ServiceMode::SystemLowRisk);
    assert!(input.request.dry_run);
    assert_eq!(input.request.unit_dir, Some(PathBuf::from("/tmp/init.d")));
    assert_eq!(
        input.request.config_dir,
        PathBuf::from("/tmp/stutter-config")
    );
    assert_eq!(input.request.state_dir, PathBuf::from("/tmp/stutter-state"));
    assert_eq!(input.request.log_dir, PathBuf::from("/tmp/stutter-log"));
}

#[test]
fn parses_service_uninstall_dry_run_command() {
    let command = parse_service_command([
        "stutter",
        "service",
        "uninstall",
        "--mode",
        "user-observe",
        "--manager",
        "systemd-user",
        "--unit-dir",
        "/tmp/user-units",
        "--binary",
        "/tmp/stutter-bin",
        "--dry-run",
        "--json",
    ])
    .unwrap();

    let AppCommand::Service(input) = command else {
        panic!("expected service command");
    };

    assert!(input.json);
    assert_eq!(input.request.action, ServiceAction::Uninstall);
    assert_eq!(input.request.manager, ServiceManager::SystemdUser);
    assert_eq!(input.request.mode, ServiceMode::UserObserve);
    assert!(input.request.dry_run);
    assert_eq!(
        input.request.unit_dir,
        Some(PathBuf::from("/tmp/user-units"))
    );
    assert_eq!(input.request.binary_path, PathBuf::from("/tmp/stutter-bin"));
}

#[test]
fn parses_service_doctor_as_dry_run_plan() {
    let command =
        parse_service_command(["stutter", "service", "doctor", "--mode", "agent"]).unwrap();

    let AppCommand::Service(input) = command else {
        panic!("expected service command");
    };

    assert!(!input.json);
    assert_eq!(input.request.action, ServiceAction::Doctor);
    assert_eq!(input.request.manager, ServiceManager::SystemdSystem);
    assert_eq!(input.request.mode, ServiceMode::Agent);
    assert!(input.request.dry_run);
}

#[test]
fn parses_service_doctor_json_and_manager() {
    let command = parse_service_command([
        "stutter",
        "service",
        "doctor",
        "--mode",
        "system-observe",
        "--manager",
        "openrc",
        "--json",
    ])
    .unwrap();

    let AppCommand::Service(input) = command else {
        panic!("expected service command");
    };

    assert!(input.json);
    assert_eq!(input.request.action, ServiceAction::Doctor);
    assert_eq!(input.request.manager, ServiceManager::OpenRc);
    assert_eq!(input.request.mode, ServiceMode::SystemObserve);
    assert!(input.request.dry_run);
}
