#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use crate::service::{
        ServiceAction, ServiceCommandRequest, ServiceManager, ServiceMode, build_service_plan,
        execute_service_plan,
    };

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
