use super::*;

#[derive(Args, Debug, Clone)]
pub(super) struct ServiceArgs {
    #[command(subcommand)]
    pub(super) command: ServiceCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub(super) enum ServiceCommand {
    Install(ServiceInstallArgs),
    Uninstall(ServiceUninstallArgs),
    Doctor(ServiceDoctorArgs),
}

#[derive(Args, Debug, Clone)]
pub(super) struct ServiceInstallArgs {
    #[arg(long = "mode", default_value = "system-observe")]
    pub(super) mode: String,

    #[arg(long = "manager", default_value = "systemd-system")]
    pub(super) manager: String,

    #[arg(long = "unit-dir", value_name = "DIR")]
    pub(super) unit_dir: Option<PathBuf>,

    #[arg(
        long = "config-dir",
        value_name = "DIR",
        default_value = "/etc/stutter"
    )]
    pub(super) config_dir: PathBuf,

    #[arg(
        long = "state-dir",
        value_name = "DIR",
        default_value = "/var/lib/stutter"
    )]
    pub(super) state_dir: PathBuf,

    #[arg(
        long = "log-dir",
        value_name = "DIR",
        default_value = "/var/log/stutter"
    )]
    pub(super) log_dir: PathBuf,

    #[arg(long = "binary", value_name = "PATH")]
    pub(super) binary: Option<PathBuf>,

    #[arg(long = "dry-run")]
    pub(super) dry_run: bool,

    #[arg(long = "json")]
    pub(super) json: bool,
}

#[derive(Args, Debug, Clone)]
pub(super) struct ServiceUninstallArgs {
    #[arg(long = "mode", default_value = "system-observe")]
    pub(super) mode: String,

    #[arg(long = "manager", default_value = "systemd-system")]
    pub(super) manager: String,

    #[arg(long = "unit-dir", value_name = "DIR")]
    pub(super) unit_dir: Option<PathBuf>,

    #[arg(
        long = "config-dir",
        value_name = "DIR",
        default_value = "/etc/stutter"
    )]
    pub(super) config_dir: PathBuf,

    #[arg(
        long = "state-dir",
        value_name = "DIR",
        default_value = "/var/lib/stutter"
    )]
    pub(super) state_dir: PathBuf,

    #[arg(
        long = "log-dir",
        value_name = "DIR",
        default_value = "/var/log/stutter"
    )]
    pub(super) log_dir: PathBuf,

    #[arg(long = "binary", value_name = "PATH")]
    pub(super) binary: Option<PathBuf>,

    #[arg(long = "dry-run")]
    pub(super) dry_run: bool,

    #[arg(long = "json")]
    pub(super) json: bool,
}

#[derive(Args, Debug, Clone)]
pub(super) struct ServiceDoctorArgs {
    #[arg(long = "mode", default_value = "system-observe")]
    pub(super) mode: String,

    #[arg(long = "manager", default_value = "systemd-system")]
    pub(super) manager: String,

    #[arg(long = "unit-dir", value_name = "DIR")]
    pub(super) unit_dir: Option<PathBuf>,

    #[arg(
        long = "config-dir",
        value_name = "DIR",
        default_value = "/etc/stutter"
    )]
    pub(super) config_dir: PathBuf,

    #[arg(
        long = "state-dir",
        value_name = "DIR",
        default_value = "/var/lib/stutter"
    )]
    pub(super) state_dir: PathBuf,

    #[arg(
        long = "log-dir",
        value_name = "DIR",
        default_value = "/var/log/stutter"
    )]
    pub(super) log_dir: PathBuf,

    #[arg(long = "binary", value_name = "PATH")]
    pub(super) binary: Option<PathBuf>,

    #[arg(long = "json")]
    pub(super) json: bool,
}

pub(super) struct ServiceCommandRequestInput {
    pub action: ServiceAction,
    pub manager: String,
    pub mode: String,
    pub dry_run: bool,
    pub unit_dir: Option<PathBuf>,
    pub config_dir: PathBuf,
    pub state_dir: PathBuf,
    pub log_dir: PathBuf,
    pub binary: Option<PathBuf>,
}

pub(super) fn build_service_command_request(
    input: ServiceCommandRequestInput,
) -> anyhow::Result<ServiceCommandRequest> {
    Ok(ServiceCommandRequest {
        action: input.action,
        manager: input.manager.parse::<ServiceManager>()?,
        mode: input.mode.parse::<ServiceMode>()?,
        dry_run: input.dry_run,
        unit_dir: input.unit_dir,
        config_dir: input.config_dir,
        state_dir: input.state_dir,
        log_dir: input.log_dir,
        binary_path: input.binary.unwrap_or_else(default_service_binary_path),
    })
}

#[cfg(test)]
#[path = "tests/service.rs"]
mod tests;
