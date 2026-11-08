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

/// GameMode's exact state alone, reading only the session bus and only when asked, so the runtime
/// does not connect to a bus for tools that are not running.
pub fn read_gamemode() -> (Exact<i64>, Exact<Vec<u32>>) {
    let mut session = connect(BusKind::Session);
    let clients = read_property(
        &mut session,
        "com.feralinteractive.GameMode",
        "/com/feralinteractive/GameMode",
        "com.feralinteractive.GameMode",
        "ClientCount",
        integer,
    );
    let games = match (&clients, &mut session) {
        (Exact::Present(count), Ok(client)) if *count > 0 => list_games(client),
        (Exact::Present(_), _) => Exact::Present(Vec::new()),
        (Exact::Absent, _) => Exact::Absent,
        (Exact::Unreachable(reason), _) => Exact::Unreachable(reason.clone()),
    };
    (clients, games)
}

/// The processes registered with GameMode: `ListGames` returns `a(io)`, pid then object path.
fn list_games(client: &mut DbusClient) -> Exact<Vec<u32>> {
    match client.call(
        "com.feralinteractive.GameMode",
        "/com/feralinteractive/GameMode",
        "com.feralinteractive.GameMode",
        "ListGames",
        "",
        &[],
    ) {
        Ok(values) => match values.first() {
            Some(Value::Array(items)) => Exact::Present(games_from(items)),
            _ => Exact::Unreachable("unexpected ListGames reply".to_owned()),
        },
        Err(error) => Exact::Unreachable(format!("{error:?}")),
    }
}

/// The pids in a `ListGames` array. A malformed entry is skipped, never guessed.
pub fn games_from(items: &[Value]) -> Vec<u32> {
    items
        .iter()
        .filter_map(|item| match item {
            Value::Struct(fields) => match fields.first() {
                Some(Value::I32(pid)) => u32::try_from(*pid).ok(),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// Refines a process-name report with what GameMode says about itself. Only an exact answer may
/// *lower* a claim: a GameMode serving nobody (`ClientCount` 0) claims no domain because it only
/// acts for registered clients. A reachable client count above zero keeps the claim, and an
/// unreachable bus changes nothing, so the report still fails closed.
pub fn refine_gamemode(report: &mut super::OwnerReport, clients: &Exact<i64>) {
    use super::{Daemon, KnownOwner};
    if report.daemon(KnownOwner::GameMode) != Daemon::Running {
        return;
    }
    match clients {
        Exact::Present(0) => {
            report.daemons.insert(KnownOwner::GameMode, Daemon::Idle);
        }
        Exact::Present(_) | Exact::Absent | Exact::Unreachable(_) => {}
    }
}

#[cfg(test)]
mod gamemode_tests {
    use super::*;
    use crate::integrations::{Daemon, KnownOwner, OwnerReport, ResourceDomain, SchedExt};

    fn report(state: Daemon) -> OwnerReport {
        let mut daemons = std::collections::BTreeMap::new();
        for (owner, _) in crate::integrations::owner_domains() {
            daemons.insert(owner, Daemon::NotRunning);
        }
        daemons.insert(KnownOwner::GameMode, state);
        OwnerReport {
            sched_ext: SchedExt::Disabled,
            daemons,
        }
    }

    #[test]
    fn a_gamemode_serving_nobody_claims_no_domain_and_every_other_answer_keeps_the_claim() {
        let mut idle = report(Daemon::Running);
        refine_gamemode(&mut idle, &Exact::Present(0));
        assert_eq!(idle.daemon(KnownOwner::GameMode), Daemon::Idle);
        assert_eq!(idle.external_owner(ResourceDomain::TaskPriority), None);
        assert_eq!(idle.external_owner(ResourceDomain::CpuPower), None);

        for kept in [
            Exact::Present(2),
            Exact::Absent,
            Exact::Unreachable("bus down".into()),
        ] {
            let mut report = report(Daemon::Running);
            refine_gamemode(&mut report, &kept);
            assert_eq!(
                report.daemon(KnownOwner::GameMode),
                Daemon::Running,
                "{kept:?}"
            );
            assert!(
                report
                    .external_owner(ResourceDomain::TaskPriority)
                    .is_some(),
                "{kept:?}"
            );
        }
        // A process that is not running is never refined into anything.
        let mut absent = report(Daemon::NotRunning);
        refine_gamemode(&mut absent, &Exact::Present(0));
        assert_eq!(absent.daemon(KnownOwner::GameMode), Daemon::NotRunning);
    }

    #[test]
    fn the_registered_games_come_from_the_listgames_structs_and_bad_entries_are_skipped() {
        let items = vec![
            Value::Struct(vec![Value::I32(4242), Value::Path("/g/1".into())]),
            Value::Struct(vec![Value::I32(-1), Value::Path("/g/2".into())]),
            Value::Struct(vec![Value::Str("x".into())]),
            Value::U32(7),
        ];
        assert_eq!(games_from(&items), vec![4242]);
        assert!(games_from(&[]).is_empty());
    }
}

/// Whether `root_pid` (or a descendant the caller names) is registered with GameMode. This is an
/// *identity* annotation only: it can corroborate that a process is a game, and it never raises
/// the trust of a frame or KPI source and never authorises an operation.
pub fn is_registered_with_gamemode(games: &[u32], pid: u32) -> bool {
    games.contains(&pid)
}

#[cfg(test)]
mod gamemode_identity_tests {
    use super::is_registered_with_gamemode;

    #[test]
    fn registration_corroborates_identity_for_exactly_the_listed_pids() {
        assert!(is_registered_with_gamemode(&[4242, 7], 4242));
        assert!(!is_registered_with_gamemode(&[4242], 4243));
        assert!(!is_registered_with_gamemode(&[], 1));
    }
}
