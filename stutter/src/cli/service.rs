use super::*;

#[derive(Args, Debug, Clone)]
pub struct ServiceArgs {
    #[command(subcommand)]
    pub command: ServiceCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum ServiceCommand {
    Install(ServiceInstallArgs),
    Uninstall(ServiceUninstallArgs),
    Doctor(ServiceDoctorArgs),
}

#[derive(Args, Debug, Clone)]
pub struct ServiceInstallArgs {
    #[arg(long = "mode", default_value = "system-observe")]
    pub mode: String,

    #[arg(long = "manager", default_value = "systemd-system")]
    pub manager: String,

    #[arg(long = "unit-dir", value_name = "DIR")]
    pub unit_dir: Option<PathBuf>,

    #[arg(
        long = "config-dir",
        value_name = "DIR",
        default_value = "/etc/stutter"
    )]
    pub config_dir: PathBuf,

    #[arg(
        long = "state-dir",
        value_name = "DIR",
        default_value = "/var/lib/stutter"
    )]
    pub state_dir: PathBuf,

    #[arg(
        long = "log-dir",
        value_name = "DIR",
        default_value = "/var/log/stutter"
    )]
    pub log_dir: PathBuf,

    #[arg(long = "binary", value_name = "PATH")]
    pub binary: Option<PathBuf>,

    #[arg(long = "dry-run")]
    pub dry_run: bool,

    #[arg(long = "json")]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct ServiceUninstallArgs {
    #[arg(long = "mode", default_value = "system-observe")]
    pub mode: String,

    #[arg(long = "manager", default_value = "systemd-system")]
    pub manager: String,

    #[arg(long = "unit-dir", value_name = "DIR")]
    pub unit_dir: Option<PathBuf>,

    #[arg(
        long = "config-dir",
        value_name = "DIR",
        default_value = "/etc/stutter"
    )]
    pub config_dir: PathBuf,

    #[arg(
        long = "state-dir",
        value_name = "DIR",
        default_value = "/var/lib/stutter"
    )]
    pub state_dir: PathBuf,

    #[arg(
        long = "log-dir",
        value_name = "DIR",
        default_value = "/var/log/stutter"
    )]
    pub log_dir: PathBuf,

    #[arg(long = "binary", value_name = "PATH")]
    pub binary: Option<PathBuf>,

    #[arg(long = "dry-run")]
    pub dry_run: bool,

    #[arg(long = "json")]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct ServiceDoctorArgs {
    #[arg(long = "mode", default_value = "system-observe")]
    pub mode: String,

    #[arg(long = "manager", default_value = "systemd-system")]
    pub manager: String,

    #[arg(long = "unit-dir", value_name = "DIR")]
    pub unit_dir: Option<PathBuf>,

    #[arg(
        long = "config-dir",
        value_name = "DIR",
        default_value = "/etc/stutter"
    )]
    pub config_dir: PathBuf,

    #[arg(
        long = "state-dir",
        value_name = "DIR",
        default_value = "/var/lib/stutter"
    )]
    pub state_dir: PathBuf,

    #[arg(
        long = "log-dir",
        value_name = "DIR",
        default_value = "/var/log/stutter"
    )]
    pub log_dir: PathBuf,

    #[arg(long = "binary", value_name = "PATH")]
    pub binary: Option<PathBuf>,

    #[arg(long = "json")]
    pub json: bool,
}

#[allow(clippy::too_many_arguments)]
pub fn build_service_command_request(
    action: ServiceAction,
    manager: String,
    mode: String,
    dry_run: bool,
    unit_dir: Option<PathBuf>,
    config_dir: PathBuf,
    state_dir: PathBuf,
    log_dir: PathBuf,
    binary: Option<PathBuf>,
) -> anyhow::Result<ServiceCommandRequest> {
    Ok(ServiceCommandRequest {
        action,
        manager: manager.parse::<ServiceManager>()?,
        mode: mode.parse::<ServiceMode>()?,
        dry_run,
        unit_dir,
        config_dir,
        state_dir,
        log_dir,
        binary_path: binary.unwrap_or_else(default_service_binary_path),
    })
}

#[cfg(test)]
mod tests {
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
}
