//! A catalog of optional PMU events beyond the always-tried cycles/instructions/cache/branch set
//! (package W9): software fault, migration and context-switch counts, generalized cache events,
//! and vendor uncore memory-bandwidth events whose PMU type is discovered from sysfs.
//!
//! The catalog only names and resolves events. Nothing here opens a counter, the runtime does not
//! enable any of it, and an event the host cannot name resolves to `Unavailable` with a reason,
//! never to a zero count or a guessed encoding.
use std::path::Path;

// perf_event_attr.type values (kernel ABI).
const PERF_TYPE_HW_CACHE: u32 = 3;
const PERF_TYPE_SOFTWARE: u32 = 1;

// Software event ids.
const SW_PAGE_FAULTS: u64 = 2;
const SW_CONTEXT_SWITCHES: u64 = 3;
const SW_CPU_MIGRATIONS: u64 = 4;

/// `cache | (op << 8) | (result << 16)`: L1D=0, LL=2, DTLB=3; read=0; miss=1.
const fn cache(id: u64, op: u64, result: u64) -> u64 {
    id | (op << 8) | (result << 16)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vendor {
    Intel,
    Amd,
    Other,
}

pub fn vendor_from_cpuinfo(cpuinfo: &str) -> Vendor {
    match cpuinfo
        .lines()
        .find_map(|line| line.strip_prefix("vendor_id"))
        .and_then(|rest| rest.split(':').nth(1))
        .map(str::trim)
    {
        Some("GenuineIntel") => Vendor::Intel,
        Some("AuthenticAMD") => Vendor::Amd,
        _ => Vendor::Other,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventSource {
    /// A fixed `perf_event_attr` type and config.
    Fixed { kind: u32, config: u64 },
    /// A named uncore/PMU device: the type comes from `/sys/bus/event_source/devices/<pmu>/type`
    /// and the config from its `events/<event>` file.
    Sysfs {
        pmu: &'static str,
        event: &'static str,
    },
}

#[derive(Clone, Copy, Debug)]
pub struct ArchEvent {
    pub name: &'static str,
    pub description: &'static str,
    pub source: EventSource,
    /// Only on this vendor; `None` for architecture-independent events.
    pub vendor: Option<Vendor>,
}

pub const CATALOG: &[ArchEvent] = &[
    ArchEvent {
        name: "page_faults",
        description: "software page-fault count",
        source: EventSource::Fixed {
            kind: PERF_TYPE_SOFTWARE,
            config: SW_PAGE_FAULTS,
        },
        vendor: None,
    },
    ArchEvent {
        name: "cpu_migrations",
        description: "software CPU-migration count",
        source: EventSource::Fixed {
            kind: PERF_TYPE_SOFTWARE,
            config: SW_CPU_MIGRATIONS,
        },
        vendor: None,
    },
    ArchEvent {
        name: "context_switches",
        description: "software context-switch count",
        source: EventSource::Fixed {
            kind: PERF_TYPE_SOFTWARE,
            config: SW_CONTEXT_SWITCHES,
        },
        vendor: None,
    },
    ArchEvent {
        name: "l1d_read_misses",
        description: "L1 data cache read misses",
        source: EventSource::Fixed {
            kind: PERF_TYPE_HW_CACHE,
            config: cache(0, 0, 1),
        },
        vendor: None,
    },
    ArchEvent {
        name: "llc_read_misses",
        description: "last-level cache read misses",
        source: EventSource::Fixed {
            kind: PERF_TYPE_HW_CACHE,
            config: cache(2, 0, 1),
        },
        vendor: None,
    },
    ArchEvent {
        name: "dtlb_read_misses",
        description: "data TLB read misses",
        source: EventSource::Fixed {
            kind: PERF_TYPE_HW_CACHE,
            config: cache(3, 0, 1),
        },
        vendor: None,
    },
    ArchEvent {
        name: "imc_cas_reads",
        description: "integrated memory controller read CAS commands (memory bandwidth)",
        source: EventSource::Sysfs {
            pmu: "uncore_imc_0",
            event: "cas_count_read",
        },
        vendor: Some(Vendor::Intel),
    },
    ArchEvent {
        name: "df_dram_reads",
        description: "data-fabric DRAM read traffic (memory bandwidth)",
        source: EventSource::Sysfs {
            pmu: "amd_df",
            event: "dram_channel_data_controller_0",
        },
        vendor: Some(Vendor::Amd),
    },
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolved {
    Event {
        kind: u32,
        config: u64,
    },
    /// Named but not resolvable here: wrong vendor, no such PMU, or an unreadable event file.
    Unavailable(String),
}

/// Parses an uncore event file such as `event=0x04,umask=0x03` into a perf config
/// (`event` in bits 0-7, `umask` in bits 8-15), or `None` for any other term.
fn parse_event_terms(text: &str) -> Option<u64> {
    let mut config = 0u64;
    for term in text.trim().split(',') {
        let (key, value) = term.split_once('=')?;
        let value = u64::from_str_radix(value.trim().trim_start_matches("0x"), 16).ok()?;
        config |= match key.trim() {
            "event" if value <= 0xff => value,
            "umask" if value <= 0xff => value << 8,
            _ => return None,
        };
    }
    Some(config)
}

pub fn resolve(event: &ArchEvent, vendor: Vendor, event_source_root: &Path) -> Resolved {
    if event.vendor.is_some_and(|only| only != vendor) {
        return Resolved::Unavailable(format!(
            "{} is not available on this CPU vendor",
            event.name
        ));
    }
    match event.source {
        EventSource::Fixed { kind, config } => Resolved::Event { kind, config },
        EventSource::Sysfs { pmu, event: file } => {
            let device = event_source_root.join(pmu);
            let Some(kind) = std::fs::read_to_string(device.join("type"))
                .ok()
                .and_then(|t| t.trim().parse::<u32>().ok())
            else {
                return Resolved::Unavailable(format!("PMU {pmu} is not present"));
            };
            match std::fs::read_to_string(device.join("events").join(file))
                .ok()
                .as_deref()
                .and_then(parse_event_terms)
            {
                Some(config) => Resolved::Event { kind, config },
                None => Resolved::Unavailable(format!("{pmu} has no readable event {file}")),
            }
        }
    }
}

/// Every catalog event with how it resolves on this host.
pub fn resolve_all(cpuinfo: &str, event_source_root: &Path) -> Vec<(&'static ArchEvent, Resolved)> {
    let vendor = vendor_from_cpuinfo(cpuinfo);
    CATALOG
        .iter()
        .map(|event| (event, resolve(event, vendor, event_source_root)))
        .collect()
}

#[cfg(test)]
#[path = "arch_events_tests.rs"]
mod tests;
