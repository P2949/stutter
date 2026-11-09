//! What the host's network stack offers, read-only, and what Stutter refuses to change (W11c).
//!
//! The congestion-control inventory and the per-interface queueing disciplines are observations.
//! Socket options and qdisc replacement are deliberately not operations, and this module is where
//! that is stated as typed refusals instead of an absence:
//!
//! * A socket option (`SO_BUSY_POLL`, `SO_SNDBUF`, `TCP_CONGESTION`, ...) is applied with
//!   `setsockopt` on a file descriptor the calling process owns. There is no way to set one on
//!   another process's socket from outside, so the only honest interface is a refusal that names
//!   why. The host-wide equivalents exist as the separate `NetSysctl*` operations.
//! * Replacing an interface's root qdisc discards its queued packets and statistics and can drop
//!   live connections' pacing state; the previous state cannot be restored exactly, so it is never
//!   Stutter's to do and the refusal says so.
use std::{fs, path::Path};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CongestionInventory {
    pub current: String,
    pub available: Vec<String>,
    /// What an unprivileged process may select; empty means only the default.
    pub allowed: Vec<String>,
}

fn words(path: &Path) -> Option<Vec<String>> {
    let text = fs::read_to_string(path).ok()?;
    Some(text.split_whitespace().map(str::to_owned).collect())
}

/// Reads the TCP congestion-control inventory from a (fake or real) `/proc`.
pub fn read_congestion(proc_root: &Path) -> Option<CongestionInventory> {
    let ipv4 = proc_root.join("sys/net/ipv4");
    let current = words(&ipv4.join("tcp_congestion_control"))?
        .into_iter()
        .next()?;
    let available = words(&ipv4.join("tcp_available_congestion_control"))?;
    let allowed = words(&ipv4.join("tcp_allowed_congestion_control")).unwrap_or_default();
    available.contains(&current).then_some(CongestionInventory {
        current,
        available,
        allowed,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QdiscEntry {
    pub interface: String,
    pub kind: String,
    pub handle: String,
    /// The interface's root discipline, as opposed to a child or ingress.
    pub root: bool,
}

/// Parses `tc qdisc show` text (`qdisc fq_codel 0: dev eth0 root refcnt 2 limit 10240p ...`).
/// Lines that do not have that shape are ignored, never guessed at.
pub fn parse_tc_qdiscs(text: &str) -> Vec<QdiscEntry> {
    text.lines()
        .filter_map(|line| {
            let tokens: Vec<&str> = line.split_whitespace().collect();
            let [first, kind, handle, dev, interface, rest @ ..] = tokens.as_slice() else {
                return None;
            };
            (*first == "qdisc" && *dev == "dev" && handle.ends_with(':')).then(|| QdiscEntry {
                interface: (*interface).to_owned(),
                kind: (*kind).to_owned(),
                handle: (*handle).to_owned(),
                root: rest.first() == Some(&"root"),
            })
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SocketOption {
    BusyPoll,
    SendBuffer,
    ReceiveBuffer,
    Priority,
    TcpNoDelay,
    TcpCongestion,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetRefusal {
    /// Socket options exist only on a descriptor the caller owns.
    SocketOptionNeedsOwningDescriptor { option: SocketOption },
    /// Replacing a root qdisc cannot be undone exactly.
    QdiscReplacementIsNotRestorable { interface: String },
}

impl NetRefusal {
    pub fn reason(&self) -> String {
        match self {
            Self::SocketOptionNeedsOwningDescriptor { option } => format!(
                "{option:?} is a per-socket option set with setsockopt on a descriptor the caller owns; \
                 it cannot be applied to another process's sockets, and the host-wide sysctl operations \
                 are the supported alternative"
            ),
            Self::QdiscReplacementIsNotRestorable { interface } => format!(
                "replacing the root qdisc of {interface} discards queued packets and statistics, so \
                 the previous state cannot be restored exactly"
            ),
        }
    }
}

/// Every socket option is refused; this is the one place that says so.
pub fn refuse_socket_option(option: SocketOption) -> NetRefusal {
    NetRefusal::SocketOptionNeedsOwningDescriptor { option }
}

/// Every qdisc replacement is refused, for any interface and any discipline.
pub fn refuse_qdisc_replacement(interface: &str) -> NetRefusal {
    NetRefusal::QdiscReplacementIsNotRestorable {
        interface: interface.to_owned(),
    }
}

#[cfg(test)]
#[path = "net_capability_tests.rs"]
mod tests;
