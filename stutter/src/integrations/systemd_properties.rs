//! Runtime resource-control properties on Stutter-owned units, through systemd1 (package W11d).
//!
//! `org.freedesktop.systemd1.Manager.SetUnitProperties(name, runtime, properties)` changes a unit's
//! cgroup resource controls the way `systemctl set-property --runtime` does: for the running unit
//! only, written to no unit file and gone at the next boot. This adapter plans such a change for a
//! unit whose name marks it as Stutter's own (`stutter-*`), only for five bounded u64 properties.
//!
//! The plan is sealed (it can only come from [`plan_properties`]) and records the unit's
//! *generation* (when it last became active) and each property's value at planning. Applying
//! re-reads both first: a restarted unit or a property someone else changed since planning is
//! refused with nothing written. After a write, any failure (the call, a readback, a mismatch) is
//! reconciled rather than propagated: each property is put back only if it still holds exactly
//! what this adapter wrote, and one that holds anything else is reported as an external change and
//! left alone. Restoring follows the same compare-before-restore rule. Executing needs an
//! [`OfflineAuthorization`] bound to the `UnitProperties` scope, which only a runner-issued receipt
//! can provide, so no runtime path invokes this.
use super::{
    PlanSeal,
    campaign_receipt::{AuthorizationDenied, CampaignScope},
    dbus::{DbusClient, DbusError, Value, Writer},
    scx_loader::OfflineAuthorization,
};

pub const SERVICE: &str = "org.freedesktop.systemd1";
pub const PATH: &str = "/org/freedesktop/systemd1";
pub const MANAGER: &str = "org.freedesktop.systemd1.Manager";
const UNIT_INTERFACE: &str = "org.freedesktop.systemd1.Unit";
const GENERATION_PROPERTY: &str = "ActiveEnterTimestampMonotonic";

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

/// A validated change. Its fields can be read but the plan cannot be built outside
/// [`plan_properties`].
///
/// ```compile_fail
/// use stutter::integrations::systemd_properties::PropertyPlan;
/// let forged = PropertyPlan {
///     unit: "sshd.service".into(),
///     changes: vec![],
///     restore: vec![],
///     generation: 1,
/// };
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PropertyPlan {
    pub unit: String,
    pub changes: Vec<(UnitProperty, u64)>,
    /// What each changed property held at planning, to write back on restore.
    pub restore: Vec<(UnitProperty, u64)>,
    /// The unit's `ActiveEnterTimestampMonotonic` at planning: a restart changes it.
    pub generation: u64,
    seal: PlanSeal,
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
    /// The unit reported no generation, so a restart could not be detected.
    GenerationUnknown,
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

/// Plans a runtime change; `current` is each property's value and `generation` the unit's, both as
/// read from the unit just now.
pub fn plan_properties(
    unit: &str,
    changes: &[(UnitProperty, u64)],
    current: &[(UnitProperty, u64)],
    generation: Option<u64>,
) -> Result<PropertyPlan, PlanDenial> {
    if !owned_unit(unit) {
        return Err(PlanDenial::NotOwnedUnit);
    }
    if changes.is_empty() {
        return Err(PlanDenial::EmptyChange);
    }
    let generation = generation
        .filter(|g| *g > 0)
        .ok_or(PlanDenial::GenerationUnknown)?;
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
        generation,
        seal: PlanSeal::issue(),
    })
}

#[derive(Debug)]
pub enum PropertyError {
    Dbus(DbusError),
    /// The authorization does not cover unit properties; nothing was sent to the bus.
    Unauthorized(AuthorizationDenied),
    /// The unit was restarted since planning; nothing was written (or, on restore, nothing was
    /// touched, because a restart already reset its runtime properties).
    UnitChanged {
        planned: u64,
        found: Option<u64>,
    },
    /// A property no longer holds the value it had at planning; nothing was written.
    StaleBaseline {
        property: UnitProperty,
        planned: u64,
        found: Option<u64>,
    },
    /// A property did not read back as written; every property still holding what this adapter
    /// wrote was put back.
    NotInEffect {
        property: UnitProperty,
        wanted: u64,
        found: Option<u64>,
        unresolved: Vec<UnitProperty>,
    },
    /// The write or a read failed part-way. Each property was reconciled; any that someone else
    /// changed is listed and was left alone.
    PartialFailure {
        error: DbusError,
        unresolved: Vec<UnitProperty>,
    },
    /// Someone else changed these properties after this adapter did, so they were not restored.
    ExternalChange {
        unresolved: Vec<UnitProperty>,
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

fn u64_of(value: Value) -> Option<u64> {
    match value {
        Value::U64(value) => Some(value),
        _ => None,
    }
}

/// Reads one property as it is now (`None` when the unit does not report a u64 for it).
pub fn read_property(
    client: &mut DbusClient,
    unit: &str,
    property: UnitProperty,
) -> Result<Option<u64>, DbusError> {
    let path = unit_path(client, unit)?;
    Ok(u64_of(client.get_property(
        SERVICE,
        &path,
        interface_of(unit),
        property.name(),
    )?))
}

/// The unit's generation: when it last became active, in monotonic microseconds.
pub fn read_generation(client: &mut DbusClient, unit: &str) -> Result<Option<u64>, DbusError> {
    let path = unit_path(client, unit)?;
    Ok(u64_of(client.get_property(
        SERVICE,
        &path,
        UNIT_INTERFACE,
        GENERATION_PROPERTY,
    )?))
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

/// Puts back, among the changed properties, exactly those that still hold what this adapter wrote;
/// returns the ones that hold anything else (someone else's change), untouched.
fn reconcile(client: &mut DbusClient, plan: &PropertyPlan) -> Result<Vec<UnitProperty>, DbusError> {
    let mut to_write = Vec::new();
    let mut unresolved = Vec::new();
    for ((property, applied), (_, baseline)) in plan.changes.iter().zip(&plan.restore) {
        let found = read_property(client, &plan.unit, *property)?;
        if found == Some(*baseline) {
            continue;
        }
        if found == Some(*applied) {
            to_write.push((*property, *baseline));
        } else {
            unresolved.push(*property);
        }
    }
    if !to_write.is_empty() {
        set(client, &plan.unit, &to_write)?;
        for (property, baseline) in &to_write {
            if read_property(client, &plan.unit, *property)? != Some(*baseline) {
                unresolved.push(*property);
            }
        }
    }
    Ok(unresolved)
}

/// Fails unless the unit is the one that was planned and every changed property still holds its
/// planned value.
fn precommit(client: &mut DbusClient, plan: &PropertyPlan) -> Result<(), PropertyError> {
    let found = read_generation(client, &plan.unit)?;
    if found != Some(plan.generation) {
        return Err(PropertyError::UnitChanged {
            planned: plan.generation,
            found,
        });
    }
    for (property, planned) in &plan.restore {
        let found = read_property(client, &plan.unit, *property)?;
        if found != Some(*planned) {
            return Err(PropertyError::StaleBaseline {
                property: *property,
                planned: *planned,
                found,
            });
        }
    }
    Ok(())
}

/// Applies the plan at runtime and verifies every property. Any failure after the write is
/// reconciled by compare-before-restore, never by overwriting someone else's value.
pub fn apply(
    client: &mut DbusClient,
    plan: &PropertyPlan,
    authorization: &OfflineAuthorization,
) -> Result<(), PropertyError> {
    authorization
        .require(CampaignScope::UnitProperties)
        .map_err(PropertyError::Unauthorized)?;
    precommit(client, plan)?;
    let outcome = set(client, &plan.unit, &plan.changes).and_then(|()| {
        for (property, wanted) in &plan.changes {
            let found = read_property(client, &plan.unit, *property)?;
            if found != Some(*wanted) {
                return Ok(Some((*property, *wanted, found)));
            }
        }
        Ok(None)
    });
    match outcome {
        Ok(None) => Ok(()),
        Ok(Some((property, wanted, found))) => Err(PropertyError::NotInEffect {
            property,
            wanted,
            found,
            unresolved: reconcile(client, plan)
                .unwrap_or_else(|_| plan.changes.iter().map(|(property, _)| *property).collect()),
        }),
        Err(error) => Err(PropertyError::PartialFailure {
            error,
            unresolved: reconcile(client, plan)
                .unwrap_or_else(|_| plan.changes.iter().map(|(property, _)| *property).collect()),
        }),
    }
}

/// Writes the captured values back, property by property and only where this adapter's own value is
/// still in place. A restarted unit already lost its runtime properties, so nothing is written.
pub fn restore(
    client: &mut DbusClient,
    plan: &PropertyPlan,
    authorization: &OfflineAuthorization,
) -> Result<(), PropertyError> {
    authorization
        .require_for_restore(CampaignScope::UnitProperties)
        .map_err(PropertyError::Unauthorized)?;
    let found = read_generation(client, &plan.unit)?;
    if found != Some(plan.generation) {
        return Err(PropertyError::UnitChanged {
            planned: plan.generation,
            found,
        });
    }
    let unresolved = reconcile(client, plan)?;
    if unresolved.is_empty() {
        Ok(())
    } else {
        Err(PropertyError::ExternalChange { unresolved })
    }
}

#[cfg(test)]
#[path = "systemd_properties_tests.rs"]
mod tests;
