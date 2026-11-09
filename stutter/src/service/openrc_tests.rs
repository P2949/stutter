//! Rendered OpenRC hooks tested without an init daemon or host service changes.
use std::{fs, os::unix::fs::PermissionsExt, process::Command};

use super::*;

#[test]
fn bounded_openrc_has_fixed_independent_identity_and_observation_policy() {
    let root = crate::test_support::TestRoot::new("bounded-openrc");
    let user = std::env::var("USER").unwrap();
    let mut plan = build_service_plan(ServiceCommandRequest {
        controller_user: Some(user.clone()),
        action: ServiceAction::Install,
        mode: ServiceMode::SystemBoundedAutotune,
        manager: ServiceManager::OpenRc,
        dry_run: false,
        unit_dir: Some(root.join("init.d")),
        config_dir: root.join("config"),
        state_dir: "/var/lib/stutter".into(),
        log_dir: "/var/log/stutter".into(),
        binary_path: "/run/current-system/sw/bin/true".into(),
    });
    execute_service_plan(&plan).unwrap();
    assert_eq!(plan.unit_name, format!("stutter-autotune-bounded.{user}"));
    let worker_path = plan
        .unit_target
        .with_file_name(format!("stutter-privileged-worker.{user}"));
    assert!(plan.steps.iter().any(|step| step.path == worker_path));
    let controller = fs::read_to_string(&plan.unit_target).unwrap();
    let worker = fs::read_to_string(&worker_path).unwrap();
    assert!(controller.contains("--mode observe"));
    assert!(!controller.contains("--allow-medium-risk"));
    assert!(worker.contains("STUTTER_SERVICE_BACKEND=openrc"));
    assert!(worker.contains("\"${stutter_cap_last}\" -le 40"));
    let caps = worker
        .lines()
        .find_map(|line| line.strip_prefix("capabilities=\""))
        .unwrap()
        .trim_end_matches('"');
    let fields: Vec<_> = caps.split(',').collect();
    assert_eq!(fields.len(), 41);
    for capability in 0..=40 {
        let allowed = match capability {
            0 => Some("^cap_chown"),
            1 => Some("^cap_dac_override"),
            19 => Some("^cap_sys_ptrace"),
            23 => Some("^cap_sys_nice"),
            _ => None,
        };
        assert!(
            fields.contains(
                &allowed
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("!{capability}"))
                    .as_str()
            )
        );
    }
    for (path, template) in [(&plan.unit_target, &controller), (&worker_path, &worker)] {
        assert!(!template.contains("@CONTROLLER_USER@"));
        assert!(!template.contains("@BINARY@"));
        assert!(template.contains("supervisor=supervise-daemon"));
        assert!(template.contains("respawn_max=5"));
        assert!(template.contains("no_new_privs=yes"));
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert!(
            Command::new("sh")
                .arg("-n")
                .arg(path)
                .status()
                .unwrap()
                .success()
        );
        // Source the installed hooks with side-effecting init helpers replaced. In particular,
        // a preloaded conf.d identity cannot silently change the installed workload account.
        let script = r#"
            stutter_controller_user=root
            . "$1"
            checkpath() { printf 'checkpath %s\n' "$*"; }
            need() { printf 'need %s\n' "$*"; }
            su() { printf 'su %s\n' "$*"; }
            start_pre || exit
            depend
            printf 'identity %s\npidfile %s\n' "$stutter_controller_user" "$pidfile"
            stop_post || exit
            # A failed restore hook must reach the service manager as failure.
            su() { return 73; }
            command=false
            if stop_post; then exit 74; fi
        "#;
        let output = Command::new("sh")
            .args(["-c", script, "openrc-hook-test"])
            .arg(path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output = String::from_utf8(output.stdout).unwrap();
        assert!(output.contains(&format!("identity {user}\n")));
        assert!(output.contains(&format!("pidfile /run/stutter-worker-{user}/")));
        assert!(output.contains("checkpath --directory --mode 0700"));
        if path == &plan.unit_target {
            assert!(output.contains(&format!("need localmount stutter-privileged-worker.{user}")));
            assert!(output.contains("daemon emergency-restore"));
        } else {
            assert!(output.contains("checkpath --directory --mode 0711 --owner root"));
        }
    }
    let config = plan
        .config_dir
        .join(format!("autotune-bounded-{user}.toml"));
    fs::write(&config, "# retain administrator policy\n").unwrap();
    execute_service_plan(&plan).unwrap();
    assert_eq!(
        fs::read_to_string(&config).unwrap(),
        "# retain administrator policy\n"
    );
    plan.action = ServiceAction::Uninstall;
    execute_service_plan(&plan).unwrap();
    assert!(!plan.unit_target.exists());
    assert!(!worker_path.exists());
    assert!(config.exists());
}

/// Installs the bounded OpenRC units into a scratch directory and returns (controller, worker).
fn install_scratch(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let root = crate::test_support::TestRoot::new(name);
    let plan = build_service_plan(ServiceCommandRequest {
        controller_user: Some(std::env::var("USER").unwrap()),
        action: ServiceAction::Install,
        mode: ServiceMode::SystemBoundedAutotune,
        manager: ServiceManager::OpenRc,
        dry_run: false,
        unit_dir: Some(root.join("init.d")),
        config_dir: root.join("config"),
        state_dir: "/var/lib/stutter".into(),
        log_dir: "/var/log/stutter".into(),
        binary_path: "/run/current-system/sw/bin/true".into(),
    });
    execute_service_plan(&plan).unwrap();
    let worker = plan.unit_target.with_file_name(format!(
        "stutter-privileged-worker.{}",
        std::env::var("USER").unwrap()
    ));
    // The scratch root must outlive the paths: keep it for the test process.
    std::mem::forget(root);
    (plan.unit_target, worker)
}

#[test]
fn openrc_hooks_refuse_root_and_empty_identities_and_unknown_capabilities() {
    let (controller, worker) = install_scratch("openrc-negative");
    // The installed identity is baked in after conf.d, so the hooks check the fixed account; a
    // root account (uid 0) can never be used, whatever the template says.
    for path in [&controller, &worker] {
        let text = fs::read_to_string(path).unwrap();
        assert!(
            !text.contains("systemctl"),
            "no systemd call in an OpenRC unit"
        );
        let root_variant = text.replace(
            &format!(
                "stutter_controller_user=\"{}\"",
                std::env::var("USER").unwrap()
            ),
            "stutter_controller_user=\"root\"",
        );
        let patched = path.with_extension("root");
        fs::write(&patched, root_variant).unwrap();
        let out = Command::new("sh")
            .args(["-c", ". \"$1\"; checkpath() { :; }; start_pre", "t"])
            .arg(&patched)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(64), "{}", path.display());
        let empty = path.with_extension("empty");
        fs::write(
            &empty,
            text.replace(
                &format!(
                    "stutter_controller_user=\"{}\"",
                    std::env::var("USER").unwrap()
                ),
                "stutter_controller_user=\"\"",
            ),
        )
        .unwrap();
        let out = Command::new("sh")
            .args(["-c", ". \"$1\"; checkpath() { :; }; start_pre", "t"])
            .arg(&empty)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(64), "{}", path.display());
    }
    // A kernel with a capability beyond the reviewed set refuses to start the worker.
    let text = fs::read_to_string(&worker).unwrap();
    let future_kernel = worker.with_extension("future");
    fs::write(
        &future_kernel,
        text.replace("/proc/sys/kernel/cap_last_cap", "/dev/stdin"),
    )
    .unwrap();
    let out = Command::new("sh")
        .args([
            "-c",
            ". \"$1\"; checkpath() { :; }; echo 41 | start_pre",
            "t",
        ])
        .arg(&future_kernel)
        .output()
        .unwrap();
    // `start_pre` reads stdin of the pipeline's right side: give it the value directly.
    let _ = out;
    let out = Command::new("sh")
        .args([
            "-c",
            ". \"$1\"; checkpath() { :; }; start_pre < \"$2\"",
            "t",
        ])
        .arg(&future_kernel)
        .arg({
            let value = future_kernel.with_extension("cap");
            fs::write(&value, "41\n").unwrap();
            value
        })
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(78));
}

#[test]
fn openrc_stop_hooks_restore_and_recover_before_the_service_is_considered_stopped() {
    let (controller, worker) = install_scratch("openrc-stop");
    let out = Command::new("sh")
        .args([
            "-c",
            ". \"$1\"; su() { printf 'su %s\\n' \"$*\"; }; stop_post",
            "t",
        ])
        .arg(&controller)
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("daemon emergency-restore"));
    let text = fs::read_to_string(&worker).unwrap();
    assert!(
        text.contains("--recover-only"),
        "the worker recovers journals on stop"
    );
    // A failing restore reaches the service manager as a failure of the stop hook.
    let out = Command::new("sh")
        .args(["-c", ". \"$1\"; su() { return 73; }; stop_post", "t"])
        .arg(&controller)
        .output()
        .unwrap();
    assert!(!out.status.success());
}
