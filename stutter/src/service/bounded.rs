//! Installation of the product's two bounded-service components.
use std::{fs, path::Path};

use anyhow::{Context, ensure};

use super::{ServiceAction, ServiceManager, ServicePlan};

fn safe_path(path: &Path) -> bool {
    path.is_absolute()
        && path.to_str().is_some_and(|s| {
            s.bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"/._-+".contains(&c))
        })
}

pub(super) fn execute(plan: &ServicePlan) -> anyhow::Result<()> {
    ensure!(
        plan.manager != ServiceManager::SystemdUser,
        "bounded autotune requires system services"
    );
    let user = plan
        .controller_user
        .as_deref()
        .context("bounded autotune requires --controller-user USER")?;
    crate::daemon::privilege::controller_uid(user)?;
    ensure!(
        safe_path(&plan.binary_path) && safe_path(&plan.config_dir),
        "bounded service paths must be absolute and contain only letters, digits, /._-+"
    );
    ensure!(
        plan.state_dir == Path::new("/var/lib/stutter")
            && plan.log_dir == Path::new("/var/log/stutter"),
        "bounded service uses fixed per-user state/log directories"
    );
    // The root worker must execute a protected installed binary. Installing a root unit that
    // points into a writable checkout would give that checkout's owner unrestricted root code.
    if plan.action != ServiceAction::Uninstall {
        use std::os::unix::fs::MetadataExt;
        for path in plan.binary_path.ancestors() {
            let metadata = fs::metadata(path)
                .with_context(|| format!("inspect installed service binary {}", path.display()))?;
            ensure!(
                metadata.uid() == 0 && metadata.mode() & 0o022 == 0,
                "bounded service binary and ancestors must be root-owned and not group/world writable"
            );
        }
    }
    if plan.dry_run || plan.action == ServiceAction::Doctor {
        return Ok(());
    }
    let worker_name = if plan.manager == ServiceManager::OpenRc {
        "stutter-privileged-worker"
    } else {
        "stutter-privileged-worker@.service"
    };
    let worker = plan.unit_target.with_file_name(worker_name);
    let config = plan
        .config_dir
        .join(format!("autotune-bounded-{user}.toml"));
    if plan.action == ServiceAction::Uninstall {
        for path in [&plan.unit_target, &worker] {
            match fs::remove_file(path) {
                Ok(()) => (),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.into()),
            }
        }
        return Ok(());
    }
    fs::create_dir_all(
        plan.unit_target
            .parent()
            .context("unit directory missing")?,
    )?;
    fs::create_dir_all(&plan.config_dir)?;
    let worker_template = if plan.manager == ServiceManager::OpenRc {
        include_str!("../../../packaging/openrc/stutter-privileged-worker")
    } else {
        include_str!("../../../packaging/systemd/stutter-privileged-worker@.service")
    };
    for (path, template) in [
        (
            &plan.unit_target,
            plan.mode.packaged_unit_template(plan.manager),
        ),
        (&worker, worker_template),
    ] {
        let rendered = template
            .replace(
                "@BINARY@",
                plan.binary_path
                    .to_str()
                    .context("service binary path must be UTF-8")?,
            )
            .replace(
                "@CONFIG_DIR@",
                plan.config_dir
                    .to_str()
                    .context("service config path must be UTF-8")?,
            );
        fs::write(path, rendered)?;
        if plan.manager == ServiceManager::OpenRc {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
        }
    }
    // Preserve administrator configuration on a repeat install.
    if !config.exists() {
        fs::write(
            config,
            format!(
                "[autotune]\nworker_only_mutation = true\nmanage_privileged_worker = false\nprivileged_worker_socket = \"/run/stutter-worker-{user}/worker.sock\"\n"
            ),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::{ServiceCommandRequest, ServiceMode, build_service_plan};
    #[test]
    fn bounded_install_renders_both_components_and_preserves_administrator_configuration() {
        let root = crate::test_support::TestRoot::new("bounded-install");
        let user = std::env::var("USER").unwrap();
        assert_ne!(user, "root");
        let mut plan = build_service_plan(ServiceCommandRequest {
            controller_user: Some(user.clone()),
            action: ServiceAction::Install,
            mode: ServiceMode::SystemBoundedAutotune,
            manager: ServiceManager::SystemdSystem,
            dry_run: false,
            unit_dir: Some(root.join("units")),
            config_dir: root.join("config"),
            state_dir: "/var/lib/stutter".into(),
            log_dir: "/var/log/stutter".into(),
            binary_path: "/run/current-system/sw/bin/true".into(),
        });
        execute(&plan).unwrap();
        let controller = fs::read_to_string(&plan.unit_target).unwrap();
        let worker = fs::read_to_string(
            plan.unit_target
                .with_file_name("stutter-privileged-worker@.service"),
        )
        .unwrap();
        assert!(controller.contains("User=%i"));
        assert!(controller.contains("--focus-source heuristic"));
        assert!(controller.contains("/run/current-system/sw/bin/true autotune"));
        assert!(!controller.contains("CAP_SYS_NICE"));
        assert!(controller.contains("NoNewPrivileges=true"));
        assert!(worker.contains("--controller-user %i"));
        assert!(!controller.contains("@BINARY@"));
        let config = plan
            .config_dir
            .join(format!("autotune-bounded-{user}.toml"));
        fs::write(&config, "# administrator policy\n").unwrap();
        execute(&plan).unwrap();
        assert_eq!(
            fs::read_to_string(&config).unwrap(),
            "# administrator policy\n"
        );
        plan.manager = ServiceManager::SystemdUser;
        assert!(execute(&plan).is_err());
        plan.manager = ServiceManager::SystemdSystem;
        plan.controller_user = None;
        assert!(execute(&plan).is_err());
    }
}
