//! A minimal, bounded, read-only D-Bus client (package W6).
//!
//! It exists so an owner can be identified through its own documented interface (the bus name's
//! owner, a property such as `ClientCount` or `CurrentScheduler`) instead of a process name. It
//! authenticates as the calling user (`EXTERNAL`), says hello, and calls methods with a deadline.
//! It performs no writes: the only calls offered are `GetNameOwner`, `NameHasOwner`, and
//! `org.freedesktop.DBus.Properties.Get`. Every read has a timeout and a size bound.
mod client;
#[cfg(test)]
mod tests;
mod wire;

pub use client::{BusKind, DbusClient, DbusError, session_bus_path, system_bus_path};
pub use wire::Value;
