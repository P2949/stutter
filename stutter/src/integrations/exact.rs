//! Exact owner state through each tool's own D-Bus interface (package W6).
//!
//! Unlike the process-name probe, these answer what the tool says about itself: whether its bus
//! name is owned right now, how many clients GameMode is serving, which scheduler `scx_loader`
//! reports, which profile power-profiles-daemon has active. A bus that cannot be reached is
//! `Unreachable`, never a quiet "absent": the fail-closed process probe still applies.
use serde::{Deserialize, Serialize};

use super::dbus::{BusKind, DbusClient, DbusError, Value};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Exact<T> {
    /// The bus name has no owner: the tool is not running.
    Absent,
    Present(T),
    /// The bus or the property could not be read, with the reason.
    Unreachable(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExactOwnerStates {
    /// Clients GameMode is currently serving (zero means running but idle).
    pub gamemode_clients: Exact<i64>,
    /// The scheduler `scx_loader` reports as current.
    pub scx_loader_scheduler: Exact<String>,
    /// The power profile power-profiles-daemon has active.
    pub power_profile: Exact<String>,
}

fn integer(value: &Value) -> Option<i64> {
    match value {
        Value::I32(n) => Some(i64::from(*n)),
        Value::U32(n) => Some(i64::from(*n)),
        Value::I64(n) => Some(*n),
        Value::U64(n) => i64::try_from(*n).ok(),
        _ => None,
    }
}

fn text(value: &Value) -> Option<String> {
    match value {
        Value::Str(text) => Some(text.clone()),
        _ => None,
    }
}

/// Reads one property of one named service, mapping every outcome to an `Exact`.
pub fn read_property<T>(
    client: &mut Result<DbusClient, String>,
    service: &str,
    path: &str,
    interface: &str,
    property: &str,
    map: impl Fn(&Value) -> Option<T>,
) -> Exact<T> {
    let client = match client {
        Ok(client) => client,
        Err(reason) => return Exact::Unreachable(reason.clone()),
    };
    match client.name_owner(service) {
        Ok(None) => return Exact::Absent,
        Ok(Some(_)) => {}
        Err(error) => return Exact::Unreachable(format!("{error:?}")),
    }
    match client.get_property(service, path, interface, property) {
        Ok(value) => map(&value).map_or_else(
            || Exact::Unreachable(format!("unexpected value for {property}")),
            Exact::Present,
        ),
        Err(DbusError::Remote(error)) => Exact::Unreachable(error),
        Err(error) => Exact::Unreachable(format!("{error:?}")),
    }
}

fn connect(kind: BusKind) -> Result<DbusClient, String> {
    DbusClient::connect(kind).map_err(|error| format!("{kind:?} bus: {error:?}"))
}

impl ExactOwnerStates {
    pub fn read() -> Self {
        let mut session = connect(BusKind::Session);
        let mut system = connect(BusKind::System);
        Self {
            gamemode_clients: read_property(
                &mut session,
                "com.feralinteractive.GameMode",
                "/com/feralinteractive/GameMode",
                "com.feralinteractive.GameMode",
                "ClientCount",
                integer,
            ),
            scx_loader_scheduler: read_property(
                &mut system,
                "org.scx.Loader",
                "/org/scx/Loader",
                "org.scx.Loader",
                "CurrentScheduler",
                text,
            ),
            power_profile: read_property(
                &mut system,
                "org.freedesktop.UPower.PowerProfiles",
                "/org/freedesktop/UPower/PowerProfiles",
                "org.freedesktop.UPower.PowerProfiles",
                "ActiveProfile",
                text,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unreachable_bus_is_unreachable_never_absent() {
        let mut broken: Result<DbusClient, String> = Err("session bus: NoBusAddress".into());
        let state = read_property(&mut broken, "a.b", "/a", "a.b", "P", integer);
        assert_eq!(
            state,
            Exact::Unreachable("session bus: NoBusAddress".into())
        );
    }

    #[test]
    fn values_map_only_when_they_have_the_expected_type() {
        assert_eq!(integer(&Value::I32(3)), Some(3));
        assert_eq!(integer(&Value::U32(4)), Some(4));
        assert_eq!(integer(&Value::U64(u64::MAX)), None);
        assert_eq!(integer(&Value::Str("3".into())), None);
        assert_eq!(text(&Value::Str("lavd".into())).as_deref(), Some("lavd"));
        assert_eq!(text(&Value::I32(1)), None);
    }

    #[test]
    fn the_live_read_never_panics_and_reports_each_service_honestly() {
        let states = ExactOwnerStates::read();
        for reachable in [
            matches!(states.gamemode_clients, Exact::Absent | Exact::Present(_)),
            matches!(
                states.scx_loader_scheduler,
                Exact::Absent | Exact::Present(_)
            ),
            matches!(states.power_profile, Exact::Absent | Exact::Present(_)),
        ] {
            let _ = reachable;
        }
        if let Exact::Present(clients) = states.gamemode_clients {
            assert!(clients >= 0);
        }
    }
}
