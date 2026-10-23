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

// Under the state root rather than /usr/local: that path exists, is writable by root and survives
// reboots on every layout, including NixOS where /usr/local is absent.
pub const RUNNER_INSTALL_PATH: &str = "/var/lib/stutter-validation/bin/stutter-validation-runner";
pub const STATE_ROOT: &str = "/var/lib/stutter-validation";
pub const UNIT_DIR: &str = "/etc/systemd/system";
/// Where units go when `UNIT_DIR` is read-only (NixOS): runtime only, gone after a reboot.
pub const TRANSIENT_UNIT_DIR: &str = "/run/systemd/system";
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
    /// The directory units are installed to and whether it survives a reboot. A unit directory
    /// that cannot be written (a symlink into a read-only store) falls back to the runtime one.
    pub fn unit_dir(&self) -> (PathBuf, bool) {
        let persistent = self.under(UNIT_DIR);
        if writable_directory(&persistent) {
            (persistent, true)
        } else {
            (self.under(TRANSIENT_UNIT_DIR), false)
        }
    }
    pub fn unit(&self, name: &str) -> PathBuf {
        self.unit_dir().0.join(name)
    }
}

/// Can files be created in `dir` (creating it if needed)? Probed, not guessed: a symlink into a
/// read-only filesystem looks like a directory.
fn writable_directory(dir: &Path) -> bool {
    if fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(format!(".stutter-probe-{}", std::process::id()));
    let writable = fs::File::create(&probe).is_ok();
    let _ = fs::remove_file(&probe);
    writable
}

/// Home directory of `uid` from a passwd-format file.
pub fn home_of(passwd: &str, uid: u32) -> Option<PathBuf> {
    passwd.lines().find_map(|line| {
        let fields: Vec<&str> = line.split(':').collect();
        (fields.len() >= 6 && fields[2].parse::<u32>() == Ok(uid)).then(|| PathBuf::from(fields[5]))
    })
}

/// A path safe to put in a unit file: no whitespace, quotes, backslashes or specifiers.
fn unit_safe(path: &Path) -> bool {
    path.to_str().is_some_and(|text| {
        text.starts_with('/')
            && text
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | '+'))
    })
}

/// The service keeps `ProtectHome=read-only`; the suites it launches write their results under
/// the repository's `.llm-runs` and need the owner's cargo home for offline builds.
fn runner_dropin(repository: &Path, home: Option<&Path>) -> Result<String, String> {
    let mut paths = vec![repository.join(".llm-runs")];
    if let Some(cargo) = home.map(|h| h.join(".cargo")).filter(|c| c.is_dir()) {
        paths.push(cargo);
    }
    if let Some(bad) = paths.iter().find(|p| !unit_safe(p)) {
        return Err(format!(
            "{} cannot be expressed safely in a unit file",
            bad.display()
        ));
    }
    // The leading dash lets the unit start even when a path does not exist yet.
    let joined = paths
        .iter()
        .map(|p| format!("-{}", p.display()))
        .collect::<Vec<_>>()
        .join(" ");
    Ok(format!("[Service]\nReadWritePaths={joined}\n"))
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
    /// Units survive a reboot (so the resume service can run after one).
    pub persistent_units: bool,
    /// Drop-in for the runner service: the only writable paths under the read-only home.
    pub dropin: (PathBuf, String),
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
    let mut authorization = authorization;
    let (unit_dir, persistent_units) = layout.unit_dir();
    if !persistent_units {
        // Without a resume service that outlives a reboot, rebooting would strand the campaign.
        authorization.max_reboots = 0;
    }
    let home = fs::read_to_string(layout.root.join("etc/passwd"))
        .ok()
        .and_then(|passwd| home_of(&passwd, invoker_uid));
    let dropin = (
        unit_dir
            .join("stutter-validation-runner.service.d")
            .join("10-repository.conf"),
        runner_dropin(&canonical, home.as_deref())?,
    );
    let mut units = Vec::new();
    for unit in UNITS {
        let source = canonical.join("packaging/systemd").join(unit);
        if !source.is_file() {
            return Err(format!(
                "{} is missing from the repository",
                source.display()
            ));
        }
        units.push((source, unit_dir.join(unit)));
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
        persistent_units,
        dropin,
        authorization,
    })
}

fn install_file(source: &Path, destination: &Path, mode: u32) -> std::io::Result<()> {
    install_bytes(&fs::read(source)?, destination, mode)
}

fn install_bytes(bytes: &[u8], destination: &Path, mode: u32) -> std::io::Result<()> {
    atomic_write(destination, bytes)?;
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
    install_bytes(plan.dropin.1.as_bytes(), &plan.dropin.0, 0o644)
        .map_err(|e| io("install unit drop-in", e))?;
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
        let mut steps = vec![vec!["daemon-reload"]];
        if plan.persistent_units {
            steps.push(vec!["enable", "stutter-validation-runner.service"]);
        }
        // A reinstall (new pinned binaries for the release candidate) must replace the
        // authorization the running service already loaded.
        steps.push(vec!["restart", "stutter-validation-runner.service"]);
        if plan.persistent_units {
            steps.push(vec!["enable", "stutter-validation-resume.service"]);
        }
        for arguments in steps {
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

/// Freezes the boot candidates into the authorization at install time, as root, from what the
/// bootloader reports. Whatever boot fields the (user-staged) file carried are overwritten, so
/// the campaign user cannot choose which entries may be booted.
pub fn apply_boot_discovery(
    authorization: &mut Authorization,
    adapter: &dyn crate::boot::BootAdapter,
) -> Result<(), String> {
    let entries = adapter.discover_entries()?;
    let candidates = crate::boot::distinct_candidates(&entries);
    let original = adapter.default_entry()?.or(adapter.current_entry()?);
    let mut allowed = candidates
        .iter()
        .map(|entry| entry.id.clone())
        .collect::<Vec<_>>();
    // Returning to the original entry must always be possible, even if it duplicates a kernel.
    if let Some(original) = &original
        && !allowed.contains(original)
    {
        allowed.push(original.clone());
    }
    let cap = authorization.max_reboots;
    let budget = u32::try_from(candidates.len())
        .unwrap_or(u32::MAX)
        .saturating_add(authorization.reboot_slack);
    authorization.max_reboots = budget.min(cap);
    authorization.original_boot_entry = original;
    authorization.allowed_boot_entries = allowed;
    authorization.validate_shape()
}
