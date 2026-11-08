//! TuneD's own answer about what it is applying (package W6).
//!
//! `com.redhat.tuned.control.active_profile()` returns the active profile name and `profile_mode()`
//! whether it is `auto`, `manual` or `unknown`. Both are read-only method calls on the system bus;
//! Stutter never calls `switch_profile`, `auto_profile` or `disable`. A profile TuneD is applying
//! is a resource Stutter does not take over: this reader only describes it.
use serde::{Deserialize, Serialize};

use super::{
    dbus::{DbusClient, DbusError, Value},
    exact::Exact,
};

const SERVICE: &str = "com.redhat.tuned";
const PATH: &str = "/Tuned";
const INTERFACE: &str = "com.redhat.tuned.control";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TunedState {
    /// The active profile name (`balanced`, `throughput-performance`, ...).
    pub active_profile: String,
    /// `auto`, `manual` or `unknown` as TuneD reports it.
    pub mode: String,
}

fn call_string(client: &mut DbusClient, member: &str) -> Result<String, String> {
    match client.call(SERVICE, PATH, INTERFACE, member, "", &[]) {
        Ok(values) => match values.first() {
            Some(Value::Str(text)) if text.len() <= 256 => Ok(text.clone()),
            // `profile_mode` replies `(ss)`: the mode, then a message.
            Some(Value::Struct(fields)) => match fields.first() {
                Some(Value::Str(text)) if text.len() <= 256 => Ok(text.clone()),
                _ => Err(format!("unexpected {member} reply")),
            },
            _ => Err(format!("unexpected {member} reply")),
        },
        Err(DbusError::Remote(error)) => Err(error),
        Err(error) => Err(format!("{error:?}")),
    }
}

/// TuneD's active profile and mode: `Absent` when the bus name has no owner.
pub fn read_tuned(client: &mut DbusClient) -> Exact<TunedState> {
    match client.name_owner(SERVICE) {
        Ok(None) => return Exact::Absent,
        Ok(Some(_)) => {}
        Err(error) => return Exact::Unreachable(format!("{error:?}")),
    }
    let active_profile = match call_string(client, "active_profile") {
        Ok(profile) => profile,
        Err(reason) => return Exact::Unreachable(reason),
    };
    let mode = call_string(client, "profile_mode").unwrap_or_else(|_| "unknown".to_owned());
    Exact::Present(TunedState {
        active_profile,
        mode,
    })
}

use std::{collections::BTreeSet, fs, path::PathBuf};

use super::ResourceDomain;

/// Most include levels followed, so a cyclic or deep include chain cannot loop.
const MAX_INCLUDE_DEPTH: usize = 6;
const MAX_CONF_BYTES: u64 = 256 * 1024;

/// What a TuneD profile (with its includes) manages, read from `tuned.conf` files.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedAttributes {
    /// Every section name (`cpu`, `sysctl`, `vm`, `disk`, ...), lower-cased.
    pub sections: BTreeSet<String>,
    /// Keys under `[sysctl]`.
    pub sysctl_keys: BTreeSet<String>,
    /// Profile names visited, the profile itself first.
    pub profiles: Vec<String>,
}

fn conf_path(roots: &[PathBuf], profile: &str) -> Option<PathBuf> {
    if profile.is_empty() || profile.contains('/') || profile.contains("..") {
        return None;
    }
    roots
        .iter()
        .map(|root| root.join(profile).join("tuned.conf"))
        .find(|path| path.is_file())
}

fn parse_into(text: &str, attributes: &mut ManagedAttributes) -> Vec<String> {
    let (mut section, mut includes) = (String::new(), Vec::new());
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with(['#', ';']) {
            continue;
        }
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            section = name.trim().to_ascii_lowercase();
            attributes.sections.insert(section.clone());
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if section == "main" && key.eq_ignore_ascii_case("include") {
            includes.extend(
                value
                    .split(',')
                    .map(|name| name.trim().to_owned())
                    .filter(|name| !name.is_empty()),
            );
        } else if section == "sysctl" && !key.is_empty() {
            attributes.sysctl_keys.insert(key.to_owned());
        }
    }
    includes
}

/// The attributes `profile` manages under the profile directories `roots` (`/etc/tuned`,
/// `/usr/lib/tuned`), following `include=` to a bounded depth. `None` when the profile file cannot
/// be found or read: nothing is guessed about a profile that was not read.
pub fn read_managed_attributes(roots: &[PathBuf], profile: &str) -> Option<ManagedAttributes> {
    let mut attributes = ManagedAttributes::default();
    let mut pending = vec![(profile.to_owned(), 0usize)];
    while let Some((name, depth)) = pending.pop() {
        if attributes.profiles.contains(&name) {
            continue;
        }
        let path = conf_path(roots, &name)?;
        if fs::metadata(&path).ok()?.len() > MAX_CONF_BYTES {
            return None;
        }
        let text = fs::read_to_string(&path).ok()?;
        attributes.profiles.push(name);
        let includes = parse_into(&text, &mut attributes);
        if depth < MAX_INCLUDE_DEPTH {
            pending.extend(includes.into_iter().map(|include| (include, depth + 1)));
        }
    }
    Some(attributes)
}

impl ManagedAttributes {
    /// The resource domains this profile touches. Sections not listed here (disk, audio, net, ...)
    /// claim nothing Stutter tunes.
    pub fn domains(&self) -> BTreeSet<ResourceDomain> {
        let mut domains = BTreeSet::new();
        if self.sections.contains("cpu") || self.sections.contains("scheduler") {
            domains.insert(ResourceDomain::CpuPower);
        }
        if self.sections.contains("vm") || self.sysctl_keys.iter().any(|key| key.starts_with("vm."))
        {
            domains.insert(ResourceDomain::VmKnobs);
        }
        domains
    }
}

/// What the running TuneD's active profile manages: its active profile over the system bus, then
/// that profile's `tuned.conf` files. `None` if TuneD is absent, unreachable or unreadable.
pub fn current_managed_attributes() -> Option<ManagedAttributes> {
    let mut client = DbusClient::connect(super::dbus::BusKind::System).ok()?;
    let Exact::Present(state) = read_tuned(&mut client) else {
        return None;
    };
    read_managed_attributes(
        &[PathBuf::from("/etc/tuned"), PathBuf::from("/usr/lib/tuned")],
        &state.active_profile,
    )
}

/// An offline artifact describing a whole-profile A/B: which profile is the baseline, which the
/// candidate, and exactly what the two manage differently. It is a plan an operator runs with
/// TuneD's own tools (`tuned-adm profile ...`) between paired blocks; Stutter has no TuneD engine
/// of its own, never calls `switch_profile`, and this artifact executes nothing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileExperimentArtifact {
    pub baseline: String,
    pub candidate: String,
    pub sections_only_in_baseline: BTreeSet<String>,
    pub sections_only_in_candidate: BTreeSet<String>,
    pub sysctl_only_in_baseline: BTreeSet<String>,
    pub sysctl_only_in_candidate: BTreeSet<String>,
    /// The domains whose management differs, so a reader knows what the comparison can speak to.
    pub differing_domains: BTreeSet<ResourceDomain>,
    /// The profile to return to after the experiment.
    pub restore_to: String,
}

/// Builds the artifact from the two profiles' files, or `None` if either cannot be read or they
/// are the same profile.
pub fn profile_experiment(
    roots: &[PathBuf],
    baseline: &str,
    candidate: &str,
) -> Option<ProfileExperimentArtifact> {
    if baseline == candidate {
        return None;
    }
    let (a, b) = (
        read_managed_attributes(roots, baseline)?,
        read_managed_attributes(roots, candidate)?,
    );
    let only = |x: &BTreeSet<String>, y: &BTreeSet<String>| x.difference(y).cloned().collect();
    let (domains_a, domains_b) = (a.domains(), b.domains());
    Some(ProfileExperimentArtifact {
        baseline: baseline.to_owned(),
        candidate: candidate.to_owned(),
        sections_only_in_baseline: only(&a.sections, &b.sections),
        sections_only_in_candidate: only(&b.sections, &a.sections),
        sysctl_only_in_baseline: only(&a.sysctl_keys, &b.sysctl_keys),
        sysctl_only_in_candidate: only(&b.sysctl_keys, &a.sysctl_keys),
        differing_domains: domains_a
            .symmetric_difference(&domains_b)
            .copied()
            .collect(),
        restore_to: baseline.to_owned(),
    })
}

/// Lowers TuneD's claim to `Idle` only when its active profile was read exactly and manages
/// neither CPU power nor VM knobs. A profile that could not be read changes nothing.
pub fn refine_tuned(report: &mut super::OwnerReport, attributes: Option<&ManagedAttributes>) {
    use super::{Daemon, KnownOwner};
    if report.daemon(KnownOwner::Tuned) != Daemon::Running {
        return;
    }
    if let Some(attributes) = attributes
        && attributes.domains().is_empty()
    {
        report.daemons.insert(KnownOwner::Tuned, Daemon::Idle);
    }
}

#[cfg(test)]
#[path = "tuned_tests.rs"]
mod tests;
