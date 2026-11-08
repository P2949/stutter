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

#[cfg(test)]
#[path = "tuned_tests.rs"]
mod tests;
