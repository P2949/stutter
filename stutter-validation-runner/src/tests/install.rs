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
        },
    )
    .unwrap_err();
    assert!(error.contains("regular file"), "{error}");
    assert!(Path::new(&f.root).parent().is_some());
}
