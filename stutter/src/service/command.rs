use std::{fs, path::PathBuf};

use anyhow::Context;

use super::model::*;

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

fn enable_hint(manager: ServiceManager, unit_name: &str) -> String {
    match manager {
        ServiceManager::SystemdSystem => format!("systemctl enable --now {unit_name}"),
        ServiceManager::SystemdUser => format!("systemctl --user enable --now {unit_name}"),
        ServiceManager::OpenRc => format!("rc-service {unit_name} start"),
    }
}
