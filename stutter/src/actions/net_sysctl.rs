//! Typed network sysctl writes as prepared transactions (package W11f).
//!
//! Three distinct operations, none a general network-wide sysctl writer: the TCP congestion
//! control (only a name the kernel lists as available), the TCP receive/send buffers, and the
//! busy-poll interval. Each is bound to the network namespace it was prepared in (restore refuses
//! another one), is refused while another controller (bpftune) owns the setting
//! (`UnsupportedDelegation`), and never touches unrelated network or remote-management state.
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetKnob {
    CongestionControl(String),
    TcpRmem([u64; 3]),
    TcpWmem([u64; 3]),
    BusyPollUs(u32),
}

/// Which of the three separate operations a knob belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetOperation {
    TcpCongestionControl,
    TcpBuffers,
    BusyPoll,
}

impl NetKnob {
    pub fn operation(&self) -> NetOperation {
        match self {
            Self::CongestionControl(_) => NetOperation::TcpCongestionControl,
            Self::TcpRmem(_) | Self::TcpWmem(_) => NetOperation::TcpBuffers,
            Self::BusyPollUs(_) => NetOperation::BusyPoll,
        }
    }

    pub(crate) fn file(&self) -> &'static str {
        match self {
            Self::CongestionControl(_) => "net/ipv4/tcp_congestion_control",
            Self::TcpRmem(_) => "net/ipv4/tcp_rmem",
            Self::TcpWmem(_) => "net/ipv4/tcp_wmem",
            Self::BusyPollUs(_) => "net/core/busy_poll",
        }
    }

    pub(crate) fn render(&self) -> String {
        match self {
            Self::CongestionControl(name) => name.clone(),
            Self::TcpRmem([a, b, c]) | Self::TcpWmem([a, b, c]) => format!("{a} {b} {c}"),
            Self::BusyPollUs(us) => us.to_string(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum NetDenial {
    Unavailable,
    /// The value is outside the bounded domain (unknown algorithm, unordered or huge buffers,
    /// busy-poll above the cap).
    OutOfDomain(&'static str),
    /// Another controller owns this setting and offers no stable cooperative API.
    UnsupportedDelegation(String),
    NoChange,
    NamespaceUnknown,
}

#[derive(Debug, PartialEq, Eq)]
pub enum NetCommitError {
    Write,
    /// The kernel did not report the value; the original was written back.
    ReadbackMismatch {
        restored: bool,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub enum NetRestore {
    Restored,
    AlreadyRestored,
    ExternallyChanged,
    /// The path now belongs to a different network namespace.
    UnsafeToRestore,
    Unproven,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NetSysctlRecord {
    pub path: PathBuf,
    pub original: String,
    pub applied: String,
    pub netns: String,
}

#[derive(Debug)]
pub struct NetPrepared {
    record: NetSysctlRecord,
}

#[derive(Debug)]
pub struct NetCommitted {
    record: NetSysctlRecord,
}

/// Whether two sysctl texts hold the same value, ignoring whitespace layout.
pub(crate) fn same_value(a: &str, b: &str) -> bool {
    norm(a) == norm(b)
}

fn norm(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn netns(proc_root: &Path) -> Option<String> {
    fs::read_link(proc_root.join("self/ns/net"))
        .ok()
        .map(|link| link.display().to_string())
}

fn validate(knob: &NetKnob, sys_root: &Path) -> Result<(), NetDenial> {
    const MAX_BUFFER: u64 = 64 * 1024 * 1024;
    let buffers = |[min, default, max]: [u64; 3]| {
        (4096..=MAX_BUFFER).contains(&min) && min <= default && default <= max && max <= MAX_BUFFER
    };
    match knob {
        NetKnob::CongestionControl(name) => {
            let available =
                fs::read_to_string(sys_root.join("net/ipv4/tcp_available_congestion_control"))
                    .map_err(|_| NetDenial::Unavailable)?;
            if !available.split_whitespace().any(|a| a == name) {
                return Err(NetDenial::OutOfDomain("congestion control not available"));
            }
        }
        NetKnob::TcpRmem(v) | NetKnob::TcpWmem(v) => {
            if !buffers(*v) {
                return Err(NetDenial::OutOfDomain("tcp buffer sizes"));
            }
        }
        NetKnob::BusyPollUs(us) => {
            if *us > 200 {
                return Err(NetDenial::OutOfDomain("busy_poll above 200 us"));
            }
        }
    }
    Ok(())
}

/// Validates the value, the owner and the namespace; writes nothing. `external_owner` is the name
/// of a controller (such as bpftune) that owns network sysctls here, if one is running.
pub fn prepare(
    sys_root: &Path,
    proc_root: &Path,
    knob: &NetKnob,
    external_owner: Option<&str>,
) -> Result<NetPrepared, NetDenial> {
    validate(knob, sys_root)?;
    if let Some(owner) = external_owner {
        return Err(NetDenial::UnsupportedDelegation(owner.to_owned()));
    }
    let path = sys_root.join(knob.file());
    let original = fs::read_to_string(&path).map_err(|_| NetDenial::Unavailable)?;
    let applied = knob.render();
    if norm(&original) == norm(&applied) {
        return Err(NetDenial::NoChange);
    }
    let netns = netns(proc_root).ok_or(NetDenial::NamespaceUnknown)?;
    Ok(NetPrepared {
        record: NetSysctlRecord {
            path,
            original: original.trim().to_owned(),
            applied,
            netns,
        },
    })
}

impl NetPrepared {
    pub fn commit(self) -> Result<NetCommitted, NetCommitError> {
        let record = self.record;
        fs::write(&record.path, &record.applied).map_err(|_| NetCommitError::Write)?;
        let applied =
            fs::read_to_string(&record.path).is_ok_and(|text| norm(&text) == norm(&record.applied));
        if !applied {
            let restored = fs::write(&record.path, &record.original).is_ok();
            return Err(NetCommitError::ReadbackMismatch { restored });
        }
        Ok(NetCommitted { record })
    }
}

impl NetCommitted {
    pub fn record(&self) -> NetSysctlRecord {
        self.record.clone()
    }
}

/// Restores the original only in the same network namespace and only while ours is still set.
pub fn restore_recorded(proc_root: &Path, record: &NetSysctlRecord) -> NetRestore {
    if netns(proc_root).as_deref() != Some(record.netns.as_str()) {
        return NetRestore::UnsafeToRestore;
    }
    let Ok(current) = fs::read_to_string(&record.path) else {
        return NetRestore::UnsafeToRestore;
    };
    if norm(&current) == norm(&record.original) {
        return NetRestore::AlreadyRestored;
    }
    if norm(&current) != norm(&record.applied) {
        return NetRestore::ExternallyChanged;
    }
    if let Err(error) = write_back(&record.path, &record.original) {
        let _ = error;
        return NetRestore::Unproven;
    }
    match fs::read_to_string(&record.path) {
        Ok(after) if norm(&after) == norm(&record.original) => NetRestore::Restored,
        _ => NetRestore::Unproven,
    }
}

fn write_back(path: &Path, text: &str) -> io::Result<()> {
    fs::write(path, text)
}

#[cfg(test)]
#[path = "net_sysctl_tests.rs"]
mod tests;
