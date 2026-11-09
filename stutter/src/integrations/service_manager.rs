//! Which service manager runs this host, and where one of its services' cgroup lives (W11d).
//!
//! systemd and OpenRC both put each service in a cgroup v2 directory, but name it differently:
//! systemd as `system.slice/<name>.service`, OpenRC (unified mode) as `openrc.<name>` at the
//! hierarchy root. Resource controls themselves are plain cgroup files, so the typed cgroup
//! operations work on either once the directory is resolved; this module only detects and
//! resolves, and refuses any name that could escape the hierarchy. The OpenRC layout is verified
//! against fixtures of the documented layout, not against an OpenRC host: that is a separate
//! deferred qualification.
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceManager {
    Systemd,
    OpenRc,
    /// Neither (or both, which is never guessed between).
    Unknown,
}

/// Detects the manager from its runtime directory under `run_root` (`/run`): systemd creates
/// `systemd/system` at boot, OpenRC creates `openrc`.
pub fn detect(run_root: &Path) -> ServiceManager {
    let systemd = run_root.join("systemd/system").is_dir();
    let openrc = run_root.join("openrc").is_dir();
    match (systemd, openrc) {
        (true, false) => ServiceManager::Systemd,
        (false, true) => ServiceManager::OpenRc,
        _ => ServiceManager::Unknown,
    }
}

/// A service name that cannot name a path: no separators, no `.`/`..`, bounded.
fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 120
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '@' | '.'))
}

/// The cgroup directory of `service` under `cgroup_root`, if the manager is known, the name is
/// safe and the directory exists. systemd names may carry their `.service` suffix or not.
pub fn service_cgroup(
    manager: ServiceManager,
    cgroup_root: &Path,
    service: &str,
) -> Option<PathBuf> {
    if !safe_name(service) {
        return None;
    }
    let path = match manager {
        ServiceManager::Systemd => {
            let unit = if service.ends_with(".service") {
                service.to_owned()
            } else {
                format!("{service}.service")
            };
            cgroup_root.join("system.slice").join(unit)
        }
        ServiceManager::OpenRc => cgroup_root.join(format!("openrc.{service}")),
        ServiceManager::Unknown => return None,
    };
    path.is_dir().then_some(path)
}

/// Whether a service is Stutter's own on either manager: the same `stutter-` naming rule the
/// systemd property adapter applies.
pub fn is_stutter_service(service: &str) -> bool {
    safe_name(service) && service.starts_with("stutter-") && service.len() > "stutter-".len()
}

#[cfg(test)]
#[path = "service_manager_tests.rs"]
mod tests;
