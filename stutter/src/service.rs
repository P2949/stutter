use std::{fmt, fs, path::PathBuf, str::FromStr};

use anyhow::Context;
use serde::Serialize;

pub(crate) mod autotune;
pub(crate) mod community_rules;
pub(crate) mod daemon;
pub(crate) mod profile;
pub(crate) mod recording;
pub(crate) mod report;
pub(crate) mod scenario;

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

    fn reload_hint(self) -> &'static str {
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

pub fn default_service_binary_path() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("/usr/bin/stutter"))
}

pub fn build_service_plan(request: ServiceCommandRequest) -> ServicePlan {
    let unit_name = request.mode.unit_name(request.manager).to_owned();
    let unit_source = request.mode.packaged_unit_source(request.manager);
    let unit_dir = request
        .unit_dir
        .unwrap_or_else(|| request.manager.default_unit_dir());
    let unit_target = unit_dir.join(&unit_name);
    let mut steps = Vec::new();
    let mut hints = Vec::new();
    let mut warnings = Vec::new();

    match request.action {
        ServiceAction::Install | ServiceAction::Doctor => {
            steps.push(ServicePlanStep {
                action: "create_dir",
                path: request.config_dir.clone(),
                description: "create configuration directory".to_owned(),
            });
            steps.push(ServicePlanStep {
                action: "create_dir",
                path: request.state_dir.clone(),
                description: "create persistent daemon state directory".to_owned(),
            });
            steps.push(ServicePlanStep {
                action: "create_dir",
                path: request.log_dir.clone(),
                description: "create daemon log directory".to_owned(),
            });
            steps.push(ServicePlanStep {
                action: "copy_unit",
                path: unit_target.clone(),
                description: format!("install {unit_name} from {}", unit_source.display()),
            });
            hints.push(request.manager.reload_hint().to_owned());
            hints.push(enable_hint(request.manager, &unit_name));
        }
        ServiceAction::Uninstall => {
            steps.push(ServicePlanStep {
                action: "remove_unit",
                path: unit_target.clone(),
                description: format!("remove installed unit {unit_name}"),
            });
            hints.push(request.manager.reload_hint().to_owned());
        }
    }

    if request.mode == ServiceMode::UserObserve && request.manager != ServiceManager::SystemdUser {
        warnings.push(
            "user-observe is intended for systemd-user; this plan installs the observe unit with the requested manager"
                .to_owned(),
        );
    }
    if request.mode == ServiceMode::SystemLowRisk {
        warnings.push(
            "system-low-risk can apply reversible tuning; keep restore hooks enabled and inspect policy before enabling"
                .to_owned(),
        );
    }

    ServicePlan {
        action: request.action,
        manager: request.manager,
        mode: request.mode,
        dry_run: request.dry_run,
        unit_name,
        unit_source,
        unit_target,
        config_dir: request.config_dir,
        state_dir: request.state_dir,
        log_dir: request.log_dir,
        binary_path: request.binary_path,
        steps,
        post_install_hints: hints,
        warnings,
    }
}

pub fn execute_service_plan(plan: &ServicePlan) -> anyhow::Result<()> {
    if plan.dry_run || plan.action == ServiceAction::Doctor {
        return Ok(());
    }

    match plan.action {
        ServiceAction::Install => {
            fs::create_dir_all(&plan.config_dir).with_context(|| {
                format!(
                    "failed to create service config directory {}",
                    plan.config_dir.display()
                )
            })?;
            fs::create_dir_all(&plan.state_dir).with_context(|| {
                format!(
                    "failed to create service state directory {}",
                    plan.state_dir.display()
                )
            })?;
            fs::create_dir_all(&plan.log_dir).with_context(|| {
                format!(
                    "failed to create service log directory {}",
                    plan.log_dir.display()
                )
            })?;
            if let Some(parent) = plan.unit_target.parent() {
                fs::create_dir_all(parent).with_context(|| {
                    format!("failed to create unit directory {}", parent.display())
                })?;
            }
            fs::write(
                &plan.unit_target,
                plan.mode.packaged_unit_template(plan.manager),
            )
            .with_context(|| {
                format!(
                    "failed to write embedded service unit {} to {}",
                    plan.unit_source.display(),
                    plan.unit_target.display()
                )
            })?;
            set_installed_unit_permissions(plan)?;
        }
        ServiceAction::Uninstall => {
            if plan.unit_target.exists() {
                fs::remove_file(&plan.unit_target).with_context(|| {
                    format!(
                        "failed to remove service unit {}",
                        plan.unit_target.display()
                    )
                })?;
            }
        }
        ServiceAction::Doctor => {}
    }

    Ok(())
}

pub fn diagnose_service_plan(plan: ServicePlan) -> ServiceDoctorReport {
    let checks = vec![
        ServiceDoctorCheck {
            name: "packaged_unit",
            ok: !plan
                .mode
                .packaged_unit_template(plan.manager)
                .trim()
                .is_empty(),
            detail: plan.unit_source.display().to_string(),
        },
        ServiceDoctorCheck {
            name: "binary_path",
            ok: plan.binary_path.exists(),
            detail: plan.binary_path.display().to_string(),
        },
        ServiceDoctorCheck {
            name: "unit_target_parent",
            ok: plan
                .unit_target
                .parent()
                .is_some_and(|parent| parent.exists()),
            detail: plan
                .unit_target
                .parent()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "<none>".to_owned()),
        },
        ServiceDoctorCheck {
            name: "config_dir_parent",
            ok: plan
                .config_dir
                .parent()
                .is_some_and(|parent| parent.exists()),
            detail: plan
                .config_dir
                .parent()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "<none>".to_owned()),
        },
    ];
    let ok = checks.iter().all(|check| check.ok);

    ServiceDoctorReport { plan, checks, ok }
}

#[cfg(unix)]
fn set_installed_unit_permissions(plan: &ServicePlan) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    if plan.manager != ServiceManager::OpenRc {
        return Ok(());
    }

    let mut permissions = fs::metadata(&plan.unit_target)
        .with_context(|| {
            format!(
                "failed to read installed OpenRC unit metadata {}",
                plan.unit_target.display()
            )
        })?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&plan.unit_target, permissions).with_context(|| {
        format!(
            "failed to set OpenRC unit permissions on {}",
            plan.unit_target.display()
        )
    })
}

#[cfg(not(unix))]
fn set_installed_unit_permissions(_plan: &ServicePlan) -> anyhow::Result<()> {
    Ok(())
}

fn user_systemd_dir() -> PathBuf {
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

fn enable_hint(manager: ServiceManager, unit_name: &str) -> String {
    match manager {
        ServiceManager::SystemdSystem => format!("systemctl enable --now {unit_name}"),
        ServiceManager::SystemdUser => format!("systemctl --user enable --now {unit_name}"),
        ServiceManager::OpenRc => format!("rc-service {unit_name} start"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(
        action: ServiceAction,
        manager: ServiceManager,
        mode: ServiceMode,
    ) -> ServiceCommandRequest {
        ServiceCommandRequest {
            action,
            manager,
            mode,
            dry_run: true,
            unit_dir: Some(PathBuf::from("/tmp/stutter-units")),
            config_dir: PathBuf::from("/tmp/stutter-etc"),
            state_dir: PathBuf::from("/tmp/stutter-state"),
            log_dir: PathBuf::from("/tmp/stutter-log"),
            binary_path: PathBuf::from("/usr/bin/stutter"),
        }
    }

    #[test]
    fn service_mode_maps_to_packaged_units() {
        assert_eq!(
            ServiceMode::SystemObserve.unit_name(ServiceManager::SystemdSystem),
            "stutter-autotune-observe.service"
        );
        assert_eq!(
            ServiceMode::SystemLowRisk.unit_name(ServiceManager::OpenRc),
            "stutter-autotune-low-risk"
        );
        assert_eq!(
            ServiceMode::Agent.unit_name(ServiceManager::OpenRc),
            "stutter-agent"
        );
        assert_eq!(
            ServiceMode::SystemObserve.packaged_unit_source(ServiceManager::SystemdSystem),
            PathBuf::from("embedded/systemd/stutter-autotune-observe.service")
        );
    }

    #[test]
    fn service_install_plan_contains_directories_unit_and_hints() {
        let plan = build_service_plan(request(
            ServiceAction::Install,
            ServiceManager::SystemdSystem,
            ServiceMode::SystemLowRisk,
        ));

        assert_eq!(plan.action, ServiceAction::Install);
        assert_eq!(plan.unit_name, "stutter-autotune-low-risk.service");
        assert_eq!(
            plan.unit_target,
            PathBuf::from("/tmp/stutter-units/stutter-autotune-low-risk.service")
        );
        assert!(
            plan.steps
                .iter()
                .any(|step| step.action == "copy_unit" && step.path == plan.unit_target)
        );
        assert!(
            plan.post_install_hints
                .contains(&"systemctl daemon-reload".to_owned())
        );
        assert!(!plan.warnings.is_empty());
    }

    #[test]
    fn packaged_state_changing_services_use_daemon_emergency_restore_hook() {
        for (mode, manager) in [
            (ServiceMode::Agent, ServiceManager::SystemdSystem),
            (ServiceMode::Agent, ServiceManager::OpenRc),
            (ServiceMode::SystemLowRisk, ServiceManager::SystemdSystem),
            (ServiceMode::SystemLowRisk, ServiceManager::OpenRc),
        ] {
            let unit = mode.packaged_unit_template(manager);

            assert!(unit.contains("daemon emergency-restore"));
            assert!(!unit.contains("autotune restore"));
        }
    }

    #[test]
    fn gentoo_ebuild_and_install_docs_mark_portage_packaging_as_skeleton_only() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("stutter crate should live under the repository root")
            .to_path_buf();
        let ebuild_path = root
            .join("packaging")
            .join("gentoo")
            .join("stutter-9999.ebuild");
        let install_doc_path = root.join("docs").join("INSTALL.md");
        let ebuild = std::fs::read_to_string(ebuild_path).unwrap();
        let install_doc = std::fs::read_to_string(install_doc_path).unwrap();

        assert!(
            ebuild.contains("# Packaging skeleton only."),
            "Gentoo ebuild should declare that it is a packaging skeleton"
        );
        assert!(
            ebuild.contains("This ebuild is intentionally not production-ready yet."),
            "Gentoo ebuild should not present itself as production-ready"
        );
        assert!(
            ebuild.contains("#   scripts/install-local.sh"),
            "Gentoo ebuild should point users at the supported local install path"
        );
        assert!(
            install_doc.contains("There is no production-ready distro package yet."),
            "install docs should warn that distro packaging is not production-ready"
        );
        assert!(
            install_doc.contains("the supported install path for now"),
            "install docs should identify local install scripts as the current supported path"
        );
        assert!(
            install_doc.contains("scripts/install-local.sh"),
            "install docs should point users at scripts/install-local.sh"
        );
        assert!(
            install_doc.contains("skeleton only"),
            "install docs should describe Gentoo packaging as a skeleton"
        );
    }

    #[test]
    fn gentoo_ebuild_does_not_depend_on_stutter_account_services() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("stutter crate should live under the repository root")
            .to_path_buf();
        let ebuild_path = root
            .join("packaging")
            .join("gentoo")
            .join("stutter-9999.ebuild");
        let ebuild = std::fs::read_to_string(ebuild_path).unwrap();

        assert!(!ebuild.contains("acct-group/stutter"));
        assert!(!ebuild.contains("acct-user/stutter"));

        for (mode, manager) in [
            (ServiceMode::Agent, ServiceManager::SystemdSystem),
            (ServiceMode::SystemObserve, ServiceManager::SystemdSystem),
            (ServiceMode::SystemLowRisk, ServiceManager::SystemdSystem),
            (ServiceMode::Agent, ServiceManager::OpenRc),
            (ServiceMode::SystemObserve, ServiceManager::OpenRc),
            (ServiceMode::SystemLowRisk, ServiceManager::OpenRc),
        ] {
            let unit_path = match manager {
                ServiceManager::SystemdSystem | ServiceManager::SystemdUser => root
                    .join("packaging")
                    .join("systemd")
                    .join(mode.unit_name(manager)),
                ServiceManager::OpenRc => root
                    .join("packaging")
                    .join("openrc")
                    .join(mode.unit_name(manager)),
            };
            let unit = std::fs::read_to_string(unit_path).unwrap();

            assert!(
                !unit.lines().any(|line| line.trim() == "User=stutter"),
                "{mode:?} {manager:?} should not require acct-user/stutter"
            );
            assert!(
                !unit.lines().any(|line| {
                    let line = line.trim();
                    line.starts_with("command_user=") && line.contains("stutter")
                }),
                "{mode:?} {manager:?} should not require acct-user/stutter"
            );
        }
    }

    #[test]
    fn gentoo_live_rust_ebuild_uses_live_unpack_and_cargo_target_dir() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("stutter crate should live under the repository root")
            .to_path_buf();
        let ebuild_path = root
            .join("packaging")
            .join("gentoo")
            .join("stutter-9999.ebuild");
        let ebuild = std::fs::read_to_string(ebuild_path).unwrap();

        assert!(
            ebuild.contains("src_unpack() {\n\tgit-r3_src_unpack\n\tcargo_live_src_unpack\n}"),
            "live Rust ebuild should unpack source with git-r3 and vendor Cargo dependencies during src_unpack"
        );
        assert!(
            ebuild.contains("dobin \"$(cargo_target_dir)\"/stutter"),
            "ebuild should install the stutter binary from cargo_target_dir"
        );
        assert!(
            !ebuild.contains("dobin target/release/stutter"),
            "ebuild should not hardcode target/release/stutter"
        );
    }

    #[test]
    fn packaged_systemd_system_units_pin_home_to_var_lib_stutter() {
        for mode in [
            ServiceMode::Agent,
            ServiceMode::SystemObserve,
            ServiceMode::SystemLowRisk,
        ] {
            let unit = mode.packaged_unit_template(ServiceManager::SystemdSystem);

            assert!(
                unit.contains("Environment=HOME=/var/lib/stutter"),
                "{mode:?} systemd unit should set HOME=/var/lib/stutter"
            );
        }
    }

    #[test]
    fn packaged_openrc_units_export_home_to_var_lib_stutter() {
        for mode in [
            ServiceMode::Agent,
            ServiceMode::SystemObserve,
            ServiceMode::SystemLowRisk,
        ] {
            let unit = mode.packaged_unit_template(ServiceManager::OpenRc);

            assert!(
                unit.contains(": \"${stutter_home:=/var/lib/stutter}\""),
                "{mode:?} OpenRC unit should default stutter_home to /var/lib/stutter"
            );
            assert!(
                unit.contains("export HOME=\"${stutter_home}\""),
                "{mode:?} OpenRC unit should export HOME from stutter_home"
            );
            assert!(
                unit.contains("checkpath --directory --mode 0755 \"${stutter_home}\""),
                "{mode:?} OpenRC unit should create stutter_home before start"
            );
        }
    }

    #[test]
    fn packaged_systemd_autotune_auto_focus_units_collect_foreground_window_context() {
        for mode in [ServiceMode::SystemObserve, ServiceMode::SystemLowRisk] {
            let unit = mode.packaged_unit_template(ServiceManager::SystemdSystem);
            let auto_focus_commands = unit
                .lines()
                .map(str::trim)
                .filter(|line| line.contains("--auto-focus"))
                .collect::<Vec<_>>();

            assert!(
                !auto_focus_commands.is_empty(),
                "{mode:?} systemd unit should contain at least one auto-focus command"
            );

            for command in auto_focus_commands {
                assert!(
                    command.contains("--foreground-window"),
                    "{mode:?} systemd auto-focus branch should collect foreground-window context: {command}"
                );
            }
        }
    }

    #[test]
    fn service_install_writes_embedded_systemd_unit_template() {
        let temp = tempfile::tempdir().unwrap();
        let mut request = request(
            ServiceAction::Install,
            ServiceManager::SystemdSystem,
            ServiceMode::SystemObserve,
        );
        request.dry_run = false;
        request.unit_dir = Some(temp.path().join("units"));
        request.config_dir = temp.path().join("etc");
        request.state_dir = temp.path().join("state");
        request.log_dir = temp.path().join("log");

        let plan = build_service_plan(request);
        execute_service_plan(&plan).unwrap();

        let installed_unit = fs::read_to_string(&plan.unit_target).unwrap();
        assert_eq!(
            installed_unit,
            ServiceMode::SystemObserve.packaged_unit_template(ServiceManager::SystemdSystem)
        );
        assert_eq!(
            plan.unit_source,
            PathBuf::from("embedded/systemd/stutter-autotune-observe.service")
        );
    }

    #[cfg(unix)]
    #[test]
    fn service_install_marks_openrc_unit_executable() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let mut request = request(
            ServiceAction::Install,
            ServiceManager::OpenRc,
            ServiceMode::Agent,
        );
        request.dry_run = false;
        request.unit_dir = Some(temp.path().join("init.d"));
        request.config_dir = temp.path().join("etc");
        request.state_dir = temp.path().join("state");
        request.log_dir = temp.path().join("log");

        let plan = build_service_plan(request);
        execute_service_plan(&plan).unwrap();

        let installed_unit = fs::read_to_string(&plan.unit_target).unwrap();
        assert_eq!(
            installed_unit,
            ServiceMode::Agent.packaged_unit_template(ServiceManager::OpenRc)
        );
        assert_eq!(
            fs::metadata(&plan.unit_target)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
    }

    #[test]
    fn service_uninstall_plan_removes_only_unit() {
        let plan = build_service_plan(request(
            ServiceAction::Uninstall,
            ServiceManager::OpenRc,
            ServiceMode::Agent,
        ));

        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].action, "remove_unit");
        assert_eq!(
            plan.steps[0].path,
            PathBuf::from("/tmp/stutter-units/stutter-agent")
        );
    }

    #[test]
    fn service_manager_and_mode_parse_stable_labels() {
        assert_eq!(
            "systemd-user".parse::<ServiceManager>().unwrap(),
            ServiceManager::SystemdUser
        );
        assert_eq!(
            "low-risk".parse::<ServiceMode>().unwrap(),
            ServiceMode::SystemLowRisk
        );
    }
}
