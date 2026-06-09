use std::{fmt, path::PathBuf, str::FromStr};

use serde::Serialize;

const SYSTEMD_AGENT_UNIT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../packaging/systemd/stutter-agent.service"
));
const SYSTEMD_AUTOTUNE_OBSERVE_UNIT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../packaging/systemd/stutter-autotune-observe.service"
));
const SYSTEMD_AUTOTUNE_LOW_RISK_UNIT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../packaging/systemd/stutter-autotune-low-risk.service"
));
const OPENRC_AGENT_UNIT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../packaging/openrc/stutter-agent"
));
const OPENRC_AUTOTUNE_OBSERVE_UNIT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../packaging/openrc/stutter-autotune-observe"
));
const OPENRC_AUTOTUNE_LOW_RISK_UNIT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../packaging/openrc/stutter-autotune-low-risk"
));

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ServiceAction {
    Install,
    Uninstall,
    Doctor,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ServiceManager {
    SystemdSystem,
    SystemdUser,
    OpenRc,
}

impl ServiceManager {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SystemdSystem => "systemd-system",
            Self::SystemdUser => "systemd-user",
            Self::OpenRc => "openrc",
        }
    }

    pub fn default_unit_dir(self) -> PathBuf {
        match self {
            Self::SystemdSystem => PathBuf::from("/etc/systemd/system"),
            Self::SystemdUser => user_systemd_dir(),
            Self::OpenRc => PathBuf::from("/etc/init.d"),
        }
    }

    pub fn reload_hint(self) -> &'static str {
        match self {
            Self::SystemdSystem => "systemctl daemon-reload",
            Self::SystemdUser => "systemctl --user daemon-reload",
            Self::OpenRc => "rc-update add <service> default",
        }
    }
}

impl fmt::Display for ServiceManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ServiceManager {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "systemd-system" | "systemd" => Ok(Self::SystemdSystem),
            "systemd-user" | "user" => Ok(Self::SystemdUser),
            "openrc" | "open-rc" => Ok(Self::OpenRc),
            other => anyhow::bail!(
                "unknown service manager {other:?}; expected systemd-system, systemd-user, or openrc"
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ServiceMode {
    Agent,
    UserObserve,
    SystemObserve,
    SystemLowRisk,
}

impl ServiceMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::UserObserve => "user-observe",
            Self::SystemObserve => "system-observe",
            Self::SystemLowRisk => "system-low-risk",
        }
    }

    pub fn unit_name(self, manager: ServiceManager) -> &'static str {
        match (self, manager) {
            (Self::Agent, ServiceManager::OpenRc) => "stutter-agent",
            (Self::SystemObserve | Self::UserObserve, ServiceManager::OpenRc) => {
                "stutter-autotune-observe"
            }
            (Self::SystemLowRisk, ServiceManager::OpenRc) => "stutter-autotune-low-risk",
            (Self::Agent, _) => "stutter-agent.service",
            (Self::SystemObserve | Self::UserObserve, _) => "stutter-autotune-observe.service",
            (Self::SystemLowRisk, _) => "stutter-autotune-low-risk.service",
        }
    }

    pub fn packaged_unit_source(self, manager: ServiceManager) -> PathBuf {
        let unit_family = match manager {
            ServiceManager::SystemdSystem | ServiceManager::SystemdUser => "systemd",
            ServiceManager::OpenRc => "openrc",
        };

        PathBuf::from("embedded")
            .join(unit_family)
            .join(self.unit_name(manager))
    }

    pub fn packaged_unit_template(self, manager: ServiceManager) -> &'static str {
        match (self, manager) {
            (Self::Agent, ServiceManager::OpenRc) => OPENRC_AGENT_UNIT,
            (Self::SystemObserve | Self::UserObserve, ServiceManager::OpenRc) => {
                OPENRC_AUTOTUNE_OBSERVE_UNIT
            }
            (Self::SystemLowRisk, ServiceManager::OpenRc) => OPENRC_AUTOTUNE_LOW_RISK_UNIT,
            (Self::Agent, ServiceManager::SystemdSystem | ServiceManager::SystemdUser) => {
                SYSTEMD_AGENT_UNIT
            }
            (
                Self::SystemObserve | Self::UserObserve,
                ServiceManager::SystemdSystem | ServiceManager::SystemdUser,
            ) => SYSTEMD_AUTOTUNE_OBSERVE_UNIT,
            (Self::SystemLowRisk, ServiceManager::SystemdSystem | ServiceManager::SystemdUser) => {
                SYSTEMD_AUTOTUNE_LOW_RISK_UNIT
            }
        }
    }
}

impl fmt::Display for ServiceMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ServiceMode {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "agent" => Ok(Self::Agent),
            "user-observe" => Ok(Self::UserObserve),
            "system-observe" | "observe" => Ok(Self::SystemObserve),
            "system-low-risk" | "low-risk" => Ok(Self::SystemLowRisk),
            other => anyhow::bail!(
                "unknown service mode {other:?}; expected agent, user-observe, system-observe, or system-low-risk"
            ),
        }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ServiceCommandRequest {
    pub action: ServiceAction,
    pub manager: ServiceManager,
    pub mode: ServiceMode,
    pub dry_run: bool,
    pub unit_dir: Option<PathBuf>,
    pub config_dir: PathBuf,
    pub state_dir: PathBuf,
    pub log_dir: PathBuf,
    pub binary_path: PathBuf,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ServicePlan {
    pub action: ServiceAction,
    pub manager: ServiceManager,
    pub mode: ServiceMode,
    pub dry_run: bool,
    pub unit_name: String,
    pub unit_source: PathBuf,
    pub unit_target: PathBuf,
    pub config_dir: PathBuf,
    pub state_dir: PathBuf,
    pub log_dir: PathBuf,
    pub binary_path: PathBuf,
    pub steps: Vec<ServicePlanStep>,
    pub post_install_hints: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ServicePlanStep {
    pub action: &'static str,
    pub path: PathBuf,
    pub description: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ServiceDoctorReport {
    pub plan: ServicePlan,
    pub checks: Vec<ServiceDoctorCheck>,
    pub ok: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ServiceDoctorCheck {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
}

pub fn user_systemd_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".config"))
        })
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join("systemd")
        .join("user")
}
