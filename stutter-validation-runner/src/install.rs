//! One-time installation of the runner, run by `xtask validation-campaign bootstrap` through
//! `sudo -n` (or directly as root). It installs only fixed files to fixed locations, from a
//! staged authorization it re-validates, and never accepts a path or command from its caller
//! beyond the staged file and the repository the campaign belongs to.
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Command,
};

use stutter_core::hash::sha256_hex;

use crate::{model::Authorization, state::atomic_write};

pub const RUNNER_INSTALL_PATH: &str = "/usr/local/libexec/stutter-validation-runner";
pub const STATE_ROOT: &str = "/var/lib/stutter-validation";
pub const UNIT_DIR: &str = "/etc/systemd/system";
pub const UNITS: [&str; 2] = [
    "stutter-validation-runner.service",
    "stutter-validation-resume.service",
];

/// Installation destinations under a root directory (`/` for real installs).
#[derive(Clone, Debug)]
pub struct Layout {
    pub root: PathBuf,
}

impl Layout {
    pub fn system() -> Self {
        Self {
            root: PathBuf::from("/"),
        }
    }
    fn under(&self, absolute: &str) -> PathBuf {
        self.root.join(absolute.trim_start_matches('/'))
    }
    pub fn runner(&self) -> PathBuf {
        self.under(RUNNER_INSTALL_PATH)
    }
    pub fn campaign_state(&self, campaign_id: &str) -> PathBuf {
        self.under(STATE_ROOT).join(campaign_id)
    }
    pub fn current_link(&self) -> PathBuf {
        self.under(STATE_ROOT).join("current")
    }
    pub fn unit(&self, name: &str) -> PathBuf {
        self.under(UNIT_DIR).join(name)
    }
}

/// What an install will do; computable (and testable) without touching the system.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstallPlan {
    pub runner_source: PathBuf,
    pub runner_destination: PathBuf,
    pub units: Vec<(PathBuf, PathBuf)>,
    pub authorization_destination: PathBuf,
    pub campaign_state: PathBuf,
    pub current_link: PathBuf,
    pub authorization: Authorization,
}

/// Validates every input and derives the plan. `invoker_uid` is the real user who invoked
/// sudo; the authorization must name exactly that user as owner and the staged file must be
/// theirs, so one user cannot install an authorization for another.
pub fn plan_install(
    staged: &Path,
    repository: &Path,
    runner_source: &Path,
    invoker_uid: u32,
    layout: &Layout,
) -> Result<InstallPlan, String> {
    let metadata =
        fs::symlink_metadata(staged).map_err(|e| format!("stat staged authorization: {e}"))?;
    if !metadata.is_file() || metadata.uid() != invoker_uid {
        return Err(
            "the staged authorization must be a regular file owned by the invoking user".to_owned(),
        );
    }
    let authorization: Authorization = serde_json::from_slice(
        &fs::read(staged).map_err(|e| format!("read staged authorization: {e}"))?,
    )
    .map_err(|e| format!("staged authorization is corrupt: {e}"))?;
    authorization.validate_shape()?;
    if authorization.owner_uid != invoker_uid {
        return Err("the authorization owner is not the invoking user".to_owned());
    }
    let canonical = fs::canonicalize(repository).map_err(|e| format!("repository: {e}"))?;
    if Path::new(&authorization.repository) != canonical {
        return Err("the authorization names a different repository".to_owned());
    }
    let runner_bytes = fs::read(runner_source).map_err(|e| format!("read runner: {e}"))?;
    if sha256_hex(&runner_bytes) != authorization.runner_sha256 {
        return Err("the runner binary does not match the authorized digest".to_owned());
    }
    let mut units = Vec::new();
    for unit in UNITS {
        let source = canonical.join("packaging/systemd").join(unit);
        if !source.is_file() {
            return Err(format!(
                "{} is missing from the repository",
                source.display()
            ));
        }
        units.push((source, layout.unit(unit)));
    }
    Ok(InstallPlan {
        runner_source: runner_source.to_path_buf(),
        runner_destination: layout.runner(),
        units,
        authorization_destination: layout
            .campaign_state(&authorization.campaign_id)
            .join("authorization.json"),
        campaign_state: layout.campaign_state(&authorization.campaign_id),
        current_link: layout.current_link(),
        authorization,
    })
}

fn install_file(source: &Path, destination: &Path, mode: u32) -> std::io::Result<()> {
    let bytes = fs::read(source)?;
    atomic_write(destination, &bytes)?;
    fs::set_permissions(destination, fs::Permissions::from_mode(mode))?;
    if crate::sys::effective_uid() == 0 {
        std::os::unix::fs::chown(destination, Some(0), Some(0))?;
    }
    Ok(())
}

/// Applies the plan. With `activate` it also reloads systemd and enables the services; tests and
/// dry runs install files only.
pub fn execute(plan: &InstallPlan, activate: bool) -> Result<(), String> {
    let io = |what: &str, error: std::io::Error| format!("{what}: {error}");
    install_file(&plan.runner_source, &plan.runner_destination, 0o755)
        .map_err(|e| io("install runner", e))?;
    for (source, destination) in &plan.units {
        install_file(source, destination, 0o644).map_err(|e| io("install unit", e))?;
    }
    fs::create_dir_all(&plan.campaign_state).map_err(|e| io("create state directory", e))?;
    fs::set_permissions(&plan.campaign_state, fs::Permissions::from_mode(0o700))
        .map_err(|e| io("chmod state", e))?;
    let body = serde_json::to_vec_pretty(&plan.authorization).map_err(|e| e.to_string())?;
    atomic_write(&plan.authorization_destination, &body)
        .map_err(|e| io("write authorization", e))?;
    fs::set_permissions(
        &plan.authorization_destination,
        fs::Permissions::from_mode(0o600),
    )
    .map_err(|e| io("chmod authorization", e))?;
    if crate::sys::effective_uid() == 0 {
        std::os::unix::fs::chown(&plan.campaign_state, Some(0), Some(0))
            .map_err(|e| io("chown state", e))?;
        std::os::unix::fs::chown(&plan.authorization_destination, Some(0), Some(0))
            .map_err(|e| io("chown authorization", e))?;
    }
    let _ = fs::remove_file(&plan.current_link);
    std::os::unix::fs::symlink(&plan.campaign_state, &plan.current_link)
        .map_err(|e| io("link current", e))?;
    if activate {
        for arguments in [
            vec!["daemon-reload"],
            vec!["enable", "--now", "stutter-validation-runner.service"],
            vec!["enable", "stutter-validation-resume.service"],
        ] {
            let status = Command::new("systemctl")
                .args(&arguments)
                .status()
                .map_err(|e| io("systemctl", e))?;
            if !status.success() {
                return Err(format!("systemctl {} failed", arguments.join(" ")));
            }
        }
    }
    Ok(())
}
