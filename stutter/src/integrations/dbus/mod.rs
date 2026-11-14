//! A minimal, bounded, read-only D-Bus client (package W6).
//!
//! It exists so an owner can be identified through its own documented interface (the bus name's
//! owner, a property such as `ClientCount` or `CurrentScheduler`) instead of a process name. It
//! authenticates as the calling user (`EXTERNAL`), says hello, and calls methods with a deadline.
//! It performs no writes: the only calls offered are `GetNameOwner`, `NameHasOwner`, and
//! `org.freedesktop.DBus.Properties.Get`. Every read has a timeout and a size bound.
mod client;
#[cfg(test)]
pub(crate) mod fake;
#[cfg(test)]
mod tests;
mod wire;

pub(crate) use client::connect_stream;
pub use client::{BusKind, DbusClient, DbusError, session_bus_path, system_bus_path};
pub use wire::Value;
pub(crate) use wire::Writer;

/// A wire reader over `bytes`, for tests that decode a call body.
#[cfg(test)]
pub(crate) fn fake_reader(bytes: &[u8]) -> wire::Reader<'_> {
    wire::Reader::new(bytes)
}
