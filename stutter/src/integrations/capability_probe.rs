//! A semantic capability probe for an owner's D-Bus interface (package W6).
//!
//! Where a manager has no documented way to read its state, Stutter does not scrape a command line
//! or guess. It introspects the service once, lists the properties and methods it actually
//! exposes, and says either that state is readable (naming the properties) or that the interface is
//! `Unsupported` for reading, with the reason. A method that merely *sets* something (such as
//! System76 Scheduler's `SetForegroundProcess`) is never treated as a state reader.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{
    dbus::{DbusClient, DbusError, Value},
    exact::Exact,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InterfaceSupport {
    /// The interface exposes readable properties; their names and types.
    ReadableState {
        properties: BTreeMap<String, String>,
    },
    /// The interface exists but exposes nothing to read.
    UnsupportedInterface {
        reason: String,
        methods: Vec<String>,
    },
}

/// Properties (`name -> type`) and methods of `interface` in an introspection document.
pub fn describe(xml: &str, interface: &str) -> (BTreeMap<String, String>, Vec<String>) {
    let (mut properties, mut methods) = (BTreeMap::new(), Vec::new());
    let mut inside = false;
    for line in xml.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("<interface name=\"") {
            inside = rest.split('"').next() == Some(interface);
        } else if line.starts_with("</interface>") {
            inside = false;
        } else if inside && let Some(rest) = line.strip_prefix("<method name=\"") {
            methods.push(rest.split('"').next().unwrap_or_default().to_owned());
        } else if inside && let Some(rest) = line.strip_prefix("<property name=\"") {
            let name = rest.split('"').next().unwrap_or_default().to_owned();
            let kind = line
                .split("type=\"")
                .nth(1)
                .and_then(|t| t.split('"').next())
                .unwrap_or_default()
                .to_owned();
            // Only readable properties count as state.
            if !line.contains("access=\"write\"") {
                properties.insert(name, kind);
            }
        }
    }
    (properties, methods)
}

pub fn classify(xml: &str, interface: &str) -> InterfaceSupport {
    let (properties, methods) = describe(xml, interface);
    if properties.is_empty() {
        InterfaceSupport::UnsupportedInterface {
            reason: format!("{interface} exposes no readable property"),
            methods,
        }
    } else {
        InterfaceSupport::ReadableState { properties }
    }
}

/// Introspects `service` at `path` and classifies `interface`. `Absent` when nobody owns the name.
pub fn probe_interface(
    client: &mut DbusClient,
    service: &str,
    path: &str,
    interface: &str,
) -> Exact<InterfaceSupport> {
    match client.name_owner(service) {
        Ok(None) => return Exact::Absent,
        Ok(Some(_)) => {}
        Err(error) => return Exact::Unreachable(format!("{error:?}")),
    }
    match client.call(
        service,
        path,
        "org.freedesktop.DBus.Introspectable",
        "Introspect",
        "",
        &[],
    ) {
        Ok(values) => match values.first() {
            Some(Value::Str(xml)) => Exact::Present(classify(xml, interface)),
            _ => Exact::Unreachable("unexpected Introspect reply".to_owned()),
        },
        Err(DbusError::Remote(error)) => Exact::Unreachable(error),
        Err(error) => Exact::Unreachable(format!("{error:?}")),
    }
}

/// System76 Scheduler's documented bus name, path and interface.
pub const SYSTEM76: (&str, &str, &str) = (
    "com.system76.Scheduler",
    "/com/system76/Scheduler",
    "com.system76.Scheduler",
);

#[cfg(test)]
#[path = "capability_probe_tests.rs"]
mod tests;
