//! What Stutter can truthfully say about each tool: its interface, its state and why delegation is
//! or is not offered (package W6).
//!
//! Where a tool's documented control interface is a D-Bus service, Stutter has only a read-only
//! client, so the adapter says so and refuses delegation. It never scrapes a command line to pretend
//! otherwise, and a tool that is merely running is a `Conflict` on the domains it claims, not an
//! authority Stutter may use.
use serde::{Deserialize, Serialize};

use super::{Daemon, KnownOwner, OwnerReport, ResourceDomain, SchedExt};

/// How Stutter could talk to a tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Interface {
    /// Kernel files only (sched_ext state, sysfs).
    Kernel,
    /// A documented D-Bus service; named here, not linked.
    Dbus(&'static str),
    /// Only the process name is known.
    ProcessOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AdapterStatus {
    /// Detected and usable through an exact interface.
    Available,
    /// Visible, never writable by Stutter.
    ObserveOnly,
    /// Not running; an administrator could configure Stutter to use the resource.
    DelegableManual,
    /// Running and claiming a resource Stutter would otherwise change.
    Conflict,
    Unavailable,
    /// Could not be determined; fails closed.
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterReport {
    pub owner: KnownOwner,
    pub interface: Interface,
    pub status: AdapterStatus,
    /// The first domain this owner claims, when it is in conflict.
    pub conflicting_domain: Option<ResourceDomain>,
    /// Why delegation is refused, always set: no adapter here delegates anything.
    pub delegation: &'static str,
}

const NO_DBUS_CLIENT: &str =
    "refused: the control interface is D-Bus and no verified client is linked";
const NO_ADAPTER: &str = "refused: no stable cooperative interface is implemented";

fn interface_of(owner: KnownOwner) -> Interface {
    match owner {
        KnownOwner::ScxLoader => Interface::Dbus("org.scx.Loader"),
        KnownOwner::PowerProfilesDaemon => Interface::Dbus("org.freedesktop.UPower.PowerProfiles"),
        KnownOwner::GameMode => Interface::Dbus("com.feralinteractive.GameMode"),
        KnownOwner::Tuned => Interface::Dbus("com.redhat.tuned"),
        KnownOwner::SystemdOomd => Interface::Dbus("org.freedesktop.oom1"),
        KnownOwner::Lact
        | KnownOwner::AnanicyCpp
        | KnownOwner::System76Scheduler
        | KnownOwner::AutoCpufreq
        | KnownOwner::Irqbalance
        | KnownOwner::Bpftune
        | KnownOwner::Numad
        | KnownOwner::Uresourced => Interface::ProcessOnly,
    }
}

/// One report per known owner, from the process probe and kernel state.
pub fn adapter_reports(report: &OwnerReport) -> Vec<AdapterReport> {
    super::owner_domains()
        .into_iter()
        .map(|(owner, domains)| {
            let interface = interface_of(owner);
            let daemon = report.daemon(owner);
            let conflicting_domain = (daemon != Daemon::NotRunning)
                .then(|| domains.first().copied())
                .flatten();
            let scx_without_manager = owner == KnownOwner::ScxLoader
                && daemon == Daemon::NotRunning
                && report.sched_ext == SchedExt::Disabled;
            let status = match (daemon, conflicting_domain) {
                (Daemon::Unknown, _) => AdapterStatus::Unknown,
                (Daemon::Running, Some(_)) => AdapterStatus::Conflict,
                (Daemon::Running, None) => AdapterStatus::ObserveOnly,
                (Daemon::NotRunning, _) if scx_without_manager => AdapterStatus::DelegableManual,
                (Daemon::NotRunning, _) => AdapterStatus::Unavailable,
            };
            AdapterReport {
                owner,
                interface,
                status,
                conflicting_domain,
                delegation: match interface {
                    Interface::Dbus(_) => NO_DBUS_CLIENT,
                    Interface::Kernel | Interface::ProcessOnly => NO_ADAPTER,
                },
            }
        })
        .collect()
}
