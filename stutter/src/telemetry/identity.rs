//! Canonical workload and stream identifiers.
use std::{fs::File, io::Read};

use serde::{Deserialize, Serialize};

use super::frame_source::MeasurementStage;
use crate::autotune::observation::WorkloadIdentity;

/// Why a workload identity cannot be made canonical.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IdentityDenial {
    MissingStartTicks,
    MissingExecutable,
    MissingCgroup,
    EmptyBootId,
}

/// Boot, owner, root process (with start ticks), executable, cgroup and identity hash together.
/// Two keys that differ in any field are different workloads, so a reused PID never matches.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkloadKey {
    pub boot_id: String,
    pub uid: u32,
    pub root_pid: u32,
    pub start_ticks: u64,
    pub exe_dev: u64,
    pub exe_ino: u64,
    pub cgroup: String,
    pub identity_hash: String,
}

impl WorkloadKey {
    pub fn from_identity(
        identity: &WorkloadIdentity,
        boot_id: &str,
        uid: u32,
    ) -> Result<Self, IdentityDenial> {
        if boot_id.trim().is_empty() {
            return Err(IdentityDenial::EmptyBootId);
        }
        Ok(Self {
            boot_id: boot_id.trim().to_owned(),
            uid,
            root_pid: identity.root_pid,
            start_ticks: identity
                .process_starttime_ticks
                .ok_or(IdentityDenial::MissingStartTicks)?,
            exe_dev: identity.exe_dev.ok_or(IdentityDenial::MissingExecutable)?,
            exe_ino: identity.exe_ino.ok_or(IdentityDenial::MissingExecutable)?,
            cgroup: identity
                .cgroup_path
                .clone()
                .ok_or(IdentityDenial::MissingCgroup)?,
            identity_hash: identity.stable_hash.clone(),
        })
    }
}

/// One capture of one measurement stage of one workload, in one source epoch. A truncated,
/// rotated or replaced source starts a new epoch, so frames never mix across them.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TelemetryStreamId {
    pub workload: WorkloadKey,
    pub stage: MeasurementStage,
    pub capture_id: String,
    pub source_epoch: u64,
}

impl TelemetryStreamId {
    pub fn new(
        workload: WorkloadKey,
        stage: MeasurementStage,
        capture_id: String,
        source_epoch: u64,
    ) -> Self {
        Self {
            workload,
            stage,
            capture_id,
            source_epoch,
        }
    }

    /// The same capture after its source was rotated, truncated or replaced.
    pub fn next_epoch(&self) -> Self {
        Self {
            source_epoch: self.source_epoch.saturating_add(1),
            ..self.clone()
        }
    }

    pub fn same_epoch(&self, other: &Self) -> bool {
        self == other
    }
}

/// The kernel's identifier of this boot.
pub fn read_boot_id() -> std::io::Result<String> {
    let mut text = String::new();
    File::open("/proc/sys/kernel/random/boot_id")?.read_to_string(&mut text)?;
    Ok(text.trim().to_owned())
}

/// A random 128-bit capture identifier, hex encoded.
pub fn new_capture_id() -> std::io::Result<String> {
    let mut bytes = [0u8; 16];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}
