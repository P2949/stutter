use std::{fs, os::unix::fs::PermissionsExt, path::Path};

use super::{Scratch, authorization, me};
use crate::{
    install::{Layout, UNITS, execute, plan_install},
    model::Authorization,
};

struct Fixture {
    _scratch: Scratch,
    root: std::path::PathBuf,
    repo: std::path::PathBuf,
    runner: std::path::PathBuf,
    staged: std::path::PathBuf,
}

fn fixture(mutate: impl FnOnce(&mut Authorization)) -> Fixture {
    let scratch = Scratch::new("install");
    let repo = scratch.path().join("repo");
    fs::create_dir_all(repo.join("packaging/systemd")).unwrap();
    for unit in UNITS {
        fs::write(
            repo.join("packaging/systemd").join(unit),
            format!("[Unit]\n# {unit}\n"),
        )
        .unwrap();
    }
    let runner = scratch.path().join("runner-bin");
    fs::write(&runner, b"runner-bytes").unwrap();
    let mut auth = authorization(&fs::canonicalize(&repo).unwrap().to_string_lossy());
    auth.runner_sha256 = stutter_core::hash::sha256_hex(b"runner-bytes");
    mutate(&mut auth);
    let staged = scratch.path().join("staged.json");
    fs::write(&staged, serde_json::to_vec(&auth).unwrap()).unwrap();
    let root = scratch.path().join("sysroot");
    Fixture {
        _scratch: scratch,
        root,
        repo,
        runner,
        staged,
    }
}

fn plan(f: &Fixture, uid: u32) -> Result<crate::install::InstallPlan, String> {
    plan_install(
        &f.staged,
        &f.repo,
        &f.runner,
        uid,
        &Layout {
            root: f.root.clone(),
            trusted_uid: me(),
        },
    )
}

#[test]
fn a_valid_staged_authorization_installs_fixed_files_to_fixed_places() {
    let f = fixture(|_| {});
    let plan = plan(&f, me()).unwrap();
    execute(&plan, false).unwrap();
    let layout = Layout {
        root: f.root.clone(),
        trusted_uid: me(),
    };
    assert_eq!(fs::read(layout.runner()).unwrap(), b"runner-bytes");
    assert_eq!(
        fs::metadata(layout.runner()).unwrap().permissions().mode() & 0o777,
        0o755
    );
    for unit in UNITS {
        assert!(layout.unit(unit).is_file(), "{unit}");
    }
    let state = layout.campaign_state("core-abc1234-1");
    assert_eq!(
        fs::metadata(&state).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let installed = state.join("authorization.json");
    assert_eq!(
        fs::metadata(&installed).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let loaded = Authorization::load(&installed, false).unwrap();
    assert_eq!(loaded.campaign_id, "core-abc1234-1");
    assert_eq!(fs::read_link(layout.current_link()).unwrap(), state);
}

#[test]
fn install_refuses_authorizations_that_are_not_the_invokers() {
    // Staged by someone else, or naming a different owner.
    let f = fixture(|_| {});
    assert!(plan(&f, me() + 1).unwrap_err().contains("invoking user"));
    let f = fixture(|a| a.owner_uid = me() + 1);
    assert!(
        plan(&f, me())
            .unwrap_err()
            .contains("not the invoking user")
    );
    // Root can never be the campaign owner.
    let f = fixture(|a| a.owner_uid = 0);
    assert!(plan(&f, me()).is_err());
}

#[test]
fn install_refuses_a_foreign_repository_a_tampered_runner_and_missing_units() {
    let f = fixture(|a| a.repository = "/somewhere/else".to_owned());
    assert!(plan(&f, me()).unwrap_err().contains("different repository"));
    let f = fixture(|_| {});
    fs::write(&f.runner, b"evil-runner").unwrap();
    assert!(plan(&f, me()).unwrap_err().contains("authorized digest"));
    let f = fixture(|_| {});
    fs::remove_file(f.repo.join("packaging/systemd").join(UNITS[0])).unwrap();
    assert!(plan(&f, me()).unwrap_err().contains("missing"));
}

#[test]
fn install_refuses_corrupt_or_symlinked_staged_files() {
    let f = fixture(|_| {});
    fs::write(&f.staged, b"{ not json").unwrap();
    assert!(plan(&f, me()).unwrap_err().contains("corrupt"));
    let f = fixture(|_| {});
    let link = f.staged.with_file_name("link.json");
    std::os::unix::fs::symlink(&f.staged, &link).unwrap();
    let error = plan_install(
        &link,
        &f.repo,
        &f.runner,
        me(),
        &Layout {
            root: f.root.clone(),
            trusted_uid: me(),
        },
    )
    .unwrap_err();
    assert!(error.contains("regular file"), "{error}");
    assert!(Path::new(&f.root).parent().is_some());
}

#[test]
fn a_read_only_unit_directory_installs_transient_units_and_forbids_reboots() {
    let f = fixture(|auth| auth.max_reboots = 8);
    // A file where the unit directory should be models NixOS' unwritable /etc/systemd/system.
    fs::create_dir_all(f.root.join("etc/systemd")).unwrap();
    fs::write(f.root.join("etc/systemd/system"), b"not a directory").unwrap();
    let plan = plan(&f, me()).unwrap();
    assert!(!plan.persistent_units);
    assert_eq!(
        plan.authorization.max_reboots, 0,
        "no reboot without a persistent resume service"
    );
    execute(&plan, false).unwrap();
    for unit in UNITS {
        assert!(
            f.root.join("run/systemd/system").join(unit).is_file(),
            "{unit}"
        );
    }
}

#[test]
fn a_writable_unit_directory_is_persistent_and_keeps_the_reboot_budget() {
    let f = fixture(|auth| auth.max_reboots = 8);
    let plan = plan(&f, me()).unwrap();
    assert!(plan.persistent_units);
    assert_eq!(plan.authorization.max_reboots, 8);
}

#[test]
fn the_runner_unit_gets_a_drop_in_with_only_the_repository_runs_and_cargo_home_writable() {
    let f = fixture(|_| {});
    let home = f.root.join("home/user");
    fs::create_dir_all(home.join(".cargo")).unwrap();
    fs::create_dir_all(f.root.join("etc")).unwrap();
    fs::write(
        f.root.join("etc/passwd"),
        format!(
            "root:x:0:0::/root:/bin/sh\nuser:x:{}:100::{}:/bin/sh\n",
            me(),
            home.display()
        ),
    )
    .unwrap();
    let plan = plan(&f, me()).unwrap();
    let canonical = fs::canonicalize(&f.repo).unwrap();
    let (dropin_path, dropin_body) = plan.dropin.clone().expect("a writable host gets a drop-in");
    assert!(dropin_path.ends_with("stutter-validation-runner.service.d/10-repository.conf"));
    assert_eq!(
        dropin_body,
        format!(
            "[Service]\nReadWritePaths=-{} -{}\n",
            canonical.join(".llm-runs").display(),
            home.join(".cargo").display()
        )
    );
    execute(&plan, false).unwrap();
    assert!(dropin_path.is_file());
}

#[test]
fn home_directories_come_from_passwd_and_unsafe_paths_are_refused() {
    use crate::install::home_of;
    assert_eq!(
        home_of("a:x:1000:100::/home/a:/bin/sh\n", 1000),
        Some("/home/a".into())
    );
    assert_eq!(home_of("a:x:1000:100::/home/a:/bin/sh\n", 1), None);
    let f = fixture(|_| {});
    let spaced = f.repo.parent().unwrap().join("repo with space");
    fs::rename(&f.repo, &spaced).unwrap();
    let mut auth: Authorization = serde_json::from_slice(&fs::read(&f.staged).unwrap()).unwrap();
    auth.repository = fs::canonicalize(&spaced)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    fs::write(&f.staged, serde_json::to_vec(&auth).unwrap()).unwrap();
    let error = plan_install(
        &f.staged,
        &spaced,
        &f.runner,
        me(),
        &Layout {
            root: f.root.clone(),
            trusted_uid: me(),
        },
    )
    .unwrap_err();
    assert!(error.contains("unit file"), "{error}");
}

mod externally_managed {
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::install::InstallMode;

    fn layout(f: &Fixture, trusted_uid: u32) -> Layout {
        Layout {
            root: f.root.clone(),
            trusted_uid,
        }
    }

    /// A sysroot in which the host declares both units and the persistence contract.
    fn declare(f: &Fixture, change: impl FnOnce(&mut serde_json::Value)) {
        let units = f.root.join("etc/systemd/system");
        fs::create_dir_all(&units).unwrap();
        for unit in UNITS {
            fs::write(units.join(unit), "[Unit]\n").unwrap();
        }
        let directory = f.root.join("etc/stutter-validation");
        fs::create_dir_all(&directory).unwrap();
        for dir in [f.root.join("etc"), directory.clone()] {
            fs::set_permissions(dir, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut marker = serde_json::json!({
            "contract_version": 1,
            "state_root": "/var/lib/stutter-validation",
            "services": UNITS,
            "repository": fs::canonicalize(&f.repo).unwrap(),
        });
        change(&mut marker);
        let file = directory.join("persistent-runner-v1.json");
        fs::write(&file, marker.to_string()).unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
    }

    fn mode(f: &Fixture) -> InstallMode {
        layout(f, me()).install_mode(&f.repo)
    }

    #[test]
    fn a_trusted_marker_preserves_the_reboot_budget_and_leaves_the_units_to_the_host() {
        let f = fixture(|auth| auth.max_reboots = 5);
        declare(&f, |_| {});
        assert_eq!(mode(&f), InstallMode::ExternallyManagedPersistent);
        let plan = plan(&f, me()).unwrap();
        assert_eq!(plan.mode, InstallMode::ExternallyManagedPersistent);
        assert!(plan.units.is_empty() && plan.dropin.is_none());
        assert_eq!(plan.authorization.max_reboots, 5);
        execute(&plan, false).unwrap();
        let installed = layout(&f, me());
        assert_eq!(fs::read(installed.runner()).unwrap(), b"runner-bytes");
        assert!(
            installed
                .campaign_state("core-abc1234-1")
                .join("authorization.json")
                .is_file()
        );
        // The declared units are untouched.
        for unit in UNITS {
            assert_eq!(
                fs::read_to_string(f.root.join("etc/systemd/system").join(unit)).unwrap(),
                "[Unit]\n"
            );
        }
    }

    #[test]
    fn nothing_but_a_complete_root_owned_contract_is_trusted() {
        type Change = Box<dyn FnOnce(&mut serde_json::Value)>;
        let corruptions: Vec<(&str, Change)> = vec![
            (
                "version",
                Box::new(|m| m["contract_version"] = serde_json::json!(2)),
            ),
            (
                "state root",
                Box::new(|m| m["state_root"] = serde_json::json!("/var/lib/other")),
            ),
            (
                "services",
                Box::new(|m| {
                    m["services"] = serde_json::json!(["stutter-validation-runner.service"])
                }),
            ),
            (
                "repository",
                Box::new(|m| m["repository"] = serde_json::json!("/elsewhere")),
            ),
            (
                "extra field",
                Box::new(|m| m["persistent"] = serde_json::json!(true)),
            ),
        ];
        for (name, change) in corruptions {
            let f = fixture(|_| {});
            declare(&f, change);
            assert_ne!(mode(&f), InstallMode::ExternallyManagedPersistent, "{name}");
        }
        // The file or its directory being writable by others, or owned by someone else.
        let f = fixture(|_| {});
        declare(&f, |_| {});
        let file = f
            .root
            .join("etc/stutter-validation/persistent-runner-v1.json");
        fs::set_permissions(&file, fs::Permissions::from_mode(0o664)).unwrap();
        assert_ne!(
            mode(&f),
            InstallMode::ExternallyManagedPersistent,
            "writable file"
        );
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        fs::set_permissions(
            f.root.join("etc/stutter-validation"),
            fs::Permissions::from_mode(0o775),
        )
        .unwrap();
        assert_ne!(
            mode(&f),
            InstallMode::ExternallyManagedPersistent,
            "writable directory"
        );
        fs::set_permissions(
            f.root.join("etc/stutter-validation"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        assert_eq!(mode(&f), InstallMode::ExternallyManagedPersistent);
        assert_ne!(
            layout(&f, me().wrapping_add(1)).install_mode(&f.repo),
            InstallMode::ExternallyManagedPersistent,
            "a marker owned by another user is not the trusted one"
        );
        // A missing unit definition or a missing marker is not a contract either.
        fs::remove_file(f.root.join("etc/systemd/system").join(UNITS[1])).unwrap();
        assert_ne!(
            mode(&f),
            InstallMode::ExternallyManagedPersistent,
            "missing unit"
        );
        assert_ne!(
            mode(&fixture(|_| {})),
            InstallMode::ExternallyManagedPersistent,
            "no marker"
        );
    }
}
