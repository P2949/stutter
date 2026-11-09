//! Runtime resource-control properties on Stutter-owned units, through systemd1 (package W11d).
//!
//! `org.freedesktop.systemd1.Manager.SetUnitProperties(name, runtime, properties)` changes a unit's
//! cgroup resource controls the way `systemctl set-property --runtime` does: for the running unit
//! only, written to no unit file and gone at the next boot. This adapter plans such a change for a
//! unit whose name marks it as Stutter's own (`stutter-*`), only for five bounded u64 properties,
//! captures the values the unit had, applies the change with `runtime = true`, reads each property
//! back, and can restore the captured values. Executing needs the same [`OfflineAuthorization`] as
//! the other bus adapters, which currently has a fake-bus fixture constructor only, so no runtime
//! path can invoke it. The same controls on a unit systemd does not own are the cgroup operations.
use super::{
    dbus::{DbusClient, DbusError, Value, Writer},
    scx_loader::OfflineAuthorization,
};

pub const SERVICE: &str = "org.freedesktop.systemd1";
pub const PATH: &str = "/org/freedesktop/systemd1";
pub const MANAGER: &str = "org.freedesktop.systemd1.Manager";

/// What systemd reports for an unset limit: the property's "infinity".
pub const UNSET: u64 = u64::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum UnitProperty {
    CpuWeight,
    IoWeight,
    MemoryHigh,
    MemoryMax,
    TasksMax,
}

impl UnitProperty {
    pub fn name(self) -> &'static str {
        match self {
            Self::CpuWeight => "CPUWeight",
            Self::IoWeight => "IOWeight",
            Self::MemoryHigh => "MemoryHigh",
            Self::MemoryMax => "MemoryMax",
            Self::TasksMax => "TasksMax",
        }
    }

    /// A weight is 1..=10000; a limit is any positive value; `UNSET` (infinity) is always legal
    /// because it is what restoring an unset property writes.
    fn valid(self, value: u64) -> bool {
        match self {
            Self::CpuWeight | Self::IoWeight => (1..=10_000).contains(&value) || value == UNSET,
            _ => value > 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PropertyPlan {
    pub unit: String,
    pub changes: Vec<(UnitProperty, u64)>,
    /// What each changed property held before, to write back on restore.
    pub restore: Vec<(UnitProperty, u64)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanDenial {
    /// Only units named `stutter-*` with a service, scope or slice suffix are Stutter's own.
    NotOwnedUnit,
    EmptyChange,
    DuplicateProperty(UnitProperty),
    BadValue(UnitProperty),
    /// A changed property's current value was not supplied, so it could not be restored.
    CurrentValueUnknown(UnitProperty),
}

fn owned_unit(unit: &str) -> bool {
    let Some((stem, suffix)) = unit.rsplit_once('.') else {
        return false;
    };
    stem.starts_with("stutter-")
        && stem.len() > "stutter-".len()
        && stem.len() <= 120
        && stem
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '@'))
        && matches!(suffix, "service" | "scope" | "slice")
}

/// Plans a runtime change; `current` is each property's value as read from the unit.
pub fn plan_properties(
    unit: &str,
    changes: &[(UnitProperty, u64)],
    current: &[(UnitProperty, u64)],
) -> Result<PropertyPlan, PlanDenial> {
    if !owned_unit(unit) {
        return Err(PlanDenial::NotOwnedUnit);
    }
    if changes.is_empty() {
        return Err(PlanDenial::EmptyChange);
    }
    let mut restore = Vec::new();
    for (index, (property, value)) in changes.iter().enumerate() {
        if changes[..index].iter().any(|(other, _)| other == property) {
            return Err(PlanDenial::DuplicateProperty(*property));
        }
        if !property.valid(*value) {
            return Err(PlanDenial::BadValue(*property));
        }
        let before = current
            .iter()
            .find(|(p, _)| p == property)
            .ok_or(PlanDenial::CurrentValueUnknown(*property))?;
        restore.push(*before);
    }
    Ok(PropertyPlan {
        unit: unit.to_owned(),
        changes: changes.to_vec(),
        restore,
    })
}

#[derive(Debug)]
pub enum PropertyError {
    Dbus(DbusError),
    /// A property did not read back as written; the captured values were written back.
    NotInEffect {
        property: UnitProperty,
        wanted: u64,
        found: Option<u64>,
        restored: bool,
    },
    /// After restore a property does not hold the captured value.
    NotRestored {
        property: UnitProperty,
        wanted: u64,
        found: Option<u64>,
    },
}

impl From<DbusError> for PropertyError {
    fn from(error: DbusError) -> Self {
        Self::Dbus(error)
    }
}

/// The D-Bus interface that carries a unit's resource-control properties.
fn interface_of(unit: &str) -> &'static str {
    match unit.rsplit_once('.').map(|(_, suffix)| suffix) {
        Some("scope") => "org.freedesktop.systemd1.Scope",
        Some("slice") => "org.freedesktop.systemd1.Slice",
        _ => "org.freedesktop.systemd1.Service",
    }
}

fn unit_path(client: &mut DbusClient, unit: &str) -> Result<String, DbusError> {
    let mut body = Writer::default();
    body.string(unit);
    match client
        .call(SERVICE, PATH, MANAGER, "GetUnit", "s", &body.buf)?
        .into_iter()
        .next()
    {
        Some(Value::Path(path)) => Ok(path),
        _ => Err(DbusError::UnexpectedReply),
    }
}

/// Reads one property as it is now (`None` when the unit does not report a u64 for it).
pub fn read_property(
    client: &mut DbusClient,
    unit: &str,
    property: UnitProperty,
) -> Result<Option<u64>, DbusError> {
    let path = unit_path(client, unit)?;
    Ok(
        match client.get_property(SERVICE, &path, interface_of(unit), property.name())? {
            Value::U64(value) => Some(value),
            _ => None,
        },
    )
}

fn set(
    client: &mut DbusClient,
    unit: &str,
    values: &[(UnitProperty, u64)],
) -> Result<(), DbusError> {
    let items: Vec<(&str, u64)> = values.iter().map(|(p, v)| (p.name(), *v)).collect();
    let mut body = Writer::default();
    body.string(unit);
    body.bool(true); // runtime only: nothing is written to a unit file
    body.u64_properties(&items);
    client
        .call(
            SERVICE,
            PATH,
            MANAGER,
            "SetUnitProperties",
            "sba(sv)",
            &body.buf,
        )
        .map(drop)
}

/// The first property that does not hold its wanted value, if any.
fn first_mismatch(
    client: &mut DbusClient,
    unit: &str,
    wanted: &[(UnitProperty, u64)],
) -> Result<Option<(UnitProperty, u64, Option<u64>)>, DbusError> {
    for (property, value) in wanted {
        let found = read_property(client, unit, *property)?;
        if found != Some(*value) {
            return Ok(Some((*property, *value, found)));
        }
    }
    Ok(None)
}

/// Applies the plan at runtime and verifies every property; on any mismatch the captured values
/// are written back before the error is returned.
pub fn apply(
    client: &mut DbusClient,
    plan: &PropertyPlan,
    _authorization: &OfflineAuthorization,
) -> Result<(), PropertyError> {
    set(client, &plan.unit, &plan.changes)?;
    match first_mismatch(client, &plan.unit, &plan.changes)? {
        None => Ok(()),
        Some((property, wanted, found)) => {
            let restored = restore(client, plan).is_ok();
            Err(PropertyError::NotInEffect {
                property,
                wanted,
                found,
                restored,
            })
        }
    }
}

/// Writes the captured values back and verifies each one.
pub fn restore(client: &mut DbusClient, plan: &PropertyPlan) -> Result<(), PropertyError> {
    set(client, &plan.unit, &plan.restore)?;
    match first_mismatch(client, &plan.unit, &plan.restore)? {
        None => Ok(()),
        Some((property, wanted, found)) => Err(PropertyError::NotRestored {
            property,
            wanted,
            found,
        }),
    }
}

#[cfg(test)]
#[path = "systemd_properties_tests.rs"]
mod tests;
