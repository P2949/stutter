//! A gated, offline, scoped profile hold for power-profiles-daemon (package W11d).
//!
//! power-profiles-daemon's documented cooperative interface is `HoldProfile(profile, reason,
//! application_id) -> cookie` and `ReleaseProfile(cookie)`: a hold applies a profile for as long as
//! the holder wants and the daemon drops it when the holder's bus connection closes. That makes it
//! scoped and self-releasing, unlike writing the active profile. This module reads the active
//! profile, checks the installed method signatures, plans a hold only to an allowlisted profile,
//! and executing it needs the same [`OfflineAuthorization`] the scheduler adapter uses, which only
//! an explicit offline campaign can construct. The hold is verified, and so is its release.
use std::collections::BTreeMap;

use super::{
    dbus::{DbusClient, DbusError, Value, Writer},
    scx_loader::OfflineAuthorization,
};

pub const SERVICE: &str = "org.freedesktop.UPower.PowerProfiles";
pub const PATH: &str = "/org/freedesktop/UPower/PowerProfiles";
pub const INTERFACE: &str = "org.freedesktop.UPower.PowerProfiles";

/// The only profiles a hold may request: the daemon's own three.
pub const PROFILES: [&str; 3] = ["power-saver", "balanced", "performance"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HoldPlan {
    pub profile: String,
    pub reason: String,
    pub application_id: String,
    /// The profile that was active before the hold, which must be active again after release.
    pub restore_to: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HoldDenial {
    NotAllowlisted,
    /// The profile is not one of the daemon's three.
    UnknownProfile,
    AlreadyActive,
    MethodMissing(&'static str),
    SignatureMismatch {
        method: &'static str,
        found: String,
    },
    /// The active profile could not be read, so it could not be checked after release.
    ActiveProfileUnknown,
}

pub fn read_active(client: &mut DbusClient) -> Result<String, DbusError> {
    match client.get_property(SERVICE, PATH, INTERFACE, "ActiveProfile")? {
        Value::Str(profile) => Ok(profile),
        _ => Err(DbusError::UnexpectedReply),
    }
}

/// Plans a hold of `profile`, or says exactly why not.
pub fn plan_hold(
    active: Option<&str>,
    methods: &BTreeMap<String, String>,
    profile: &str,
    reason: &str,
    application_id: &str,
    allowlist: &[String],
) -> Result<HoldPlan, HoldDenial> {
    if !PROFILES.contains(&profile) {
        return Err(HoldDenial::UnknownProfile);
    }
    if !allowlist.iter().any(|allowed| allowed == profile) {
        return Err(HoldDenial::NotAllowlisted);
    }
    let active = active.ok_or(HoldDenial::ActiveProfileUnknown)?;
    if active == profile {
        return Err(HoldDenial::AlreadyActive);
    }
    let need = |method: &'static str, signature: &str| match methods.get(method) {
        None => Err(HoldDenial::MethodMissing(method)),
        Some(found) if found != signature => Err(HoldDenial::SignatureMismatch {
            method,
            found: found.clone(),
        }),
        Some(_) => Ok(()),
    };
    need("HoldProfile", "sss")?;
    need("ReleaseProfile", "u")?;
    Ok(HoldPlan {
        profile: profile.to_owned(),
        reason: reason.to_owned(),
        application_id: application_id.to_owned(),
        restore_to: active.to_owned(),
    })
}

#[derive(Debug)]
pub enum HoldError {
    Dbus(DbusError),
    /// The call returned but the daemon does not report the held profile; the hold was released.
    NotInEffect {
        wanted: String,
        active: String,
    },
    /// After release the prior profile is not active again.
    NotRestored {
        wanted: String,
        active: String,
    },
}

impl From<DbusError> for HoldError {
    fn from(error: DbusError) -> Self {
        Self::Dbus(error)
    }
}

/// A live hold. Dropping the bus connection also releases it in the daemon; `release` does so
/// explicitly and verifies the prior profile returned.
#[derive(Debug)]
pub struct Hold {
    cookie: u32,
    restore_to: String,
}

fn release_call(client: &mut DbusClient, cookie: u32) -> Result<(), DbusError> {
    let mut body = Writer::default();
    body.u32(cookie);
    client
        .call(SERVICE, PATH, INTERFACE, "ReleaseProfile", "u", &body.buf)
        .map(drop)
}

/// Takes the planned hold and verifies it is in effect; if it is not, the hold is released at once.
pub fn acquire(
    client: &mut DbusClient,
    plan: &HoldPlan,
    _authorization: &OfflineAuthorization,
) -> Result<Hold, HoldError> {
    let mut body = Writer::default();
    body.string(&plan.profile);
    body.string(&plan.reason);
    body.string(&plan.application_id);
    let values = client.call(SERVICE, PATH, INTERFACE, "HoldProfile", "sss", &body.buf)?;
    let Some(Value::U32(cookie)) = values.first() else {
        return Err(HoldError::Dbus(DbusError::UnexpectedReply));
    };
    let hold = Hold {
        cookie: *cookie,
        restore_to: plan.restore_to.clone(),
    };
    let active = read_active(client)?;
    if active == plan.profile {
        Ok(hold)
    } else {
        release_call(client, hold.cookie)?;
        Err(HoldError::NotInEffect {
            wanted: plan.profile.clone(),
            active,
        })
    }
}

impl Hold {
    pub fn cookie(&self) -> u32 {
        self.cookie
    }

    /// Releases the hold and verifies the profile that was active before it is active again.
    pub fn release(self, client: &mut DbusClient) -> Result<(), HoldError> {
        release_call(client, self.cookie)?;
        let active = read_active(client)?;
        if active == self.restore_to {
            Ok(())
        } else {
            Err(HoldError::NotRestored {
                wanted: self.restore_to,
                active,
            })
        }
    }
}

#[cfg(test)]
#[path = "power_profiles_tests.rs"]
mod tests;
