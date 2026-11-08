//! Tool state from systemd's own manager interface (package W6).
//!
//! Some tuning tools leave no long-running process (TLP applies settings from a oneshot unit), so
//! a process-name probe cannot see them. systemd's `GetUnit` plus the unit's `ActiveState`
//! property can. This is read-only: `GetUnit` and `Properties.Get` only, on the system bus. An
//! unreachable bus changes nothing, so the report is never *lowered* by this module; it can only
//! raise a claim when a unit is exactly active.
use super::{
    Daemon, KnownOwner, OwnerReport,
    dbus::{BusKind, DbusClient, DbusError, Value, Writer},
    exact::Exact,
};

const SYSTEMD: &str = "org.freedesktop.systemd1";
const MANAGER_PATH: &str = "/org/freedesktop/systemd1";

/// The unit that runs each owner, for the owners a unit can reveal.
pub const OWNER_UNITS: &[(KnownOwner, &str)] = &[
    (KnownOwner::Tlp, "tlp.service"),
    (KnownOwner::Thermald, "thermald.service"),
    (KnownOwner::Tuned, "tuned.service"),
    (KnownOwner::Irqbalance, "irqbalance.service"),
    (KnownOwner::Bpftune, "bpftune.service"),
    (KnownOwner::Numad, "numad.service"),
    (KnownOwner::AnanicyCpp, "ananicy-cpp.service"),
    (KnownOwner::System76Scheduler, "system76-scheduler.service"),
    (KnownOwner::AutoCpufreq, "auto-cpufreq.service"),
    (
        KnownOwner::PowerProfilesDaemon,
        "power-profiles-daemon.service",
    ),
];

/// A unit name is a plain file-like name: nothing that could be mistaken for a path or a bus
/// separator is ever sent.
fn plausible_unit(unit: &str) -> bool {
    !unit.is_empty()
        && unit.len() <= 128
        && unit
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.@".contains(&byte))
}

/// `ActiveState` of `unit`: `Absent` when systemd has never loaded it (`NoSuchUnit`).
pub fn unit_active_state(client: &mut DbusClient, unit: &str) -> Exact<String> {
    if !plausible_unit(unit) {
        return Exact::Unreachable("implausible unit name".to_owned());
    }
    let mut body = Writer::default();
    body.string(unit);
    let path = match client.call(
        SYSTEMD,
        MANAGER_PATH,
        "org.freedesktop.systemd1.Manager",
        "GetUnit",
        "s",
        &body.buf,
    ) {
        Ok(values) => match values.first() {
            Some(Value::Path(path)) => path.clone(),
            _ => return Exact::Unreachable("unexpected GetUnit reply".to_owned()),
        },
        Err(DbusError::Remote(error)) if error == "org.freedesktop.systemd1.NoSuchUnit" => {
            return Exact::Absent;
        }
        Err(error) => return Exact::Unreachable(format!("{error:?}")),
    };
    match client.get_property(
        SYSTEMD,
        &path,
        "org.freedesktop.systemd1.Unit",
        "ActiveState",
    ) {
        Ok(Value::Str(state)) => Exact::Present(state),
        Ok(_) => Exact::Unreachable("unexpected ActiveState value".to_owned()),
        Err(error) => Exact::Unreachable(format!("{error:?}")),
    }
}

/// Every owner unit's state over `client`.
pub fn read_unit_states(client: &mut DbusClient) -> Vec<(KnownOwner, Exact<String>)> {
    OWNER_UNITS
        .iter()
        .map(|(owner, unit)| (*owner, unit_active_state(client, unit)))
        .collect()
}

/// The unit states over the system bus, or nothing when it cannot be reached.
pub fn read_system_unit_states() -> Vec<(KnownOwner, Exact<String>)> {
    match DbusClient::connect(BusKind::System) {
        Ok(mut client) => read_unit_states(&mut client),
        Err(_) => Vec::new(),
    }
}

/// An exactly active unit makes its owner `Running`. Nothing else changes a report: an inactive,
/// absent or unreadable unit leaves what the process probe said.
pub fn refine_with_units(report: &mut OwnerReport, states: &[(KnownOwner, Exact<String>)]) {
    for (owner, state) in states {
        let active = matches!(
            state,
            Exact::Present(state) if matches!(state.as_str(), "active" | "activating" | "reloading")
        );
        if active && matches!(report.daemon(*owner), Daemon::NotRunning | Daemon::Idle) {
            report.daemons.insert(*owner, Daemon::Running);
        }
    }
}

#[cfg(test)]
#[path = "units_tests.rs"]
mod tests;
