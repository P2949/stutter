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
