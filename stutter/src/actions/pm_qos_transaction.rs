//! `PmQosCpuLatency` as a prepared transaction (package W11d): prepare, commit, hold, restore.
//!
//! A PM QoS request is a lease, not a stored value: the kernel drops it when the descriptor
//! closes, so "restore" means closing the descriptor and proving the aggregate target stopped
//! being ours. Nothing is journaled as a value to write back, and a crash of the holder restores
//! by itself. The operation is Experimental and never autonomous: it is system-wide, and
//! `PmQosPolicy` floors, ceilings and power guards are checked at prepare and again at commit.
use std::{
    io,
    path::{Path, PathBuf},
    time::Instant,
};

use super::pm_qos::{LeaseError, PmQosDenial, PmQosLease, PmQosPolicy, PowerContext};

#[derive(Debug)]
pub enum PrepareError {
    Denied(PmQosDenial),
    /// The device is missing or unreadable: the operation is not applicable on this host.
    Unavailable(io::Error),
    /// Another requester already holds an equal or lower target: a lease would change nothing.
    AlreadySatisfied {
        effective_us: u32,
    },
}

/// Everything captured before the request: the device, the aggregate target in force and the
/// request that will be made. A prepared record can be committed once.
#[derive(Debug)]
pub struct PmQosPrepared {
    device: PathBuf,
    baseline_us: u32,
    requested_us: u32,
    policy: PmQosPolicy,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ReleaseOutcome {
    /// The aggregate target is back at the baseline: the lease is gone and nothing else moved.
    Restored { effective_us: u32 },
    /// The aggregate target no longer equals our request, but differs from the baseline because
    /// another requester is active. Our lease is provably released; theirs is not ours to touch.
    ReleasedOtherRequesterActive { effective_us: u32 },
    /// The target still equals our request after release, or could not be read: not proven.
    Unproven,
}

/// What recovery needs to prove a lease is gone: the device, the request that was made and the
/// aggregate target before it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PmQosReleaseRecord {
    pub device: PathBuf,
    pub requested_us: u32,
    pub baseline_us: u32,
}

/// A committed request, held by this process only.
#[derive(Debug)]
pub struct PmQosHeld {
    lease: PmQosLease,
    device: PathBuf,
    baseline_us: u32,
}

impl PmQosPrepared {
    pub fn baseline_us(&self) -> u32 {
        self.baseline_us
    }

    /// Re-checks the guards with fresh power facts and acquires the verified lease.
    pub fn commit(self, context: &PowerContext) -> Result<PmQosHeld, LeaseError> {
        let lease = PmQosLease::acquire(&self.device, self.requested_us, &self.policy, context)?;
        Ok(PmQosHeld {
            lease,
            device: self.device,
            baseline_us: self.baseline_us,
        })
    }
}

/// Reads the baseline and validates the request without writing anything (the dry run).
pub fn prepare(
    device: &Path,
    latency_us: u32,
    policy: &PmQosPolicy,
    context: &PowerContext,
) -> Result<PmQosPrepared, PrepareError> {
    policy
        .permit(latency_us, context)
        .map_err(PrepareError::Denied)?;
    let baseline_us = super::pm_qos::read_effective(device).map_err(PrepareError::Unavailable)?;
    if baseline_us <= latency_us {
        return Err(PrepareError::AlreadySatisfied {
            effective_us: baseline_us,
        });
    }
    Ok(PmQosPrepared {
        device: device.to_owned(),
        baseline_us,
        requested_us: latency_us,
        policy: *policy,
    })
}

impl PmQosHeld {
    pub fn requested_us(&self) -> u32 {
        self.lease.requested_us
    }

    /// The record a rollback token carries for this lease.
    pub fn release_record(&self) -> PmQosReleaseRecord {
        PmQosReleaseRecord {
            device: self.device.clone(),
            requested_us: self.lease.requested_us,
            baseline_us: self.baseline_us,
        }
    }

    /// The policy hold limit this lease was acquired under.
    pub fn max_hold(&self) -> std::time::Duration {
        self.lease.max_hold()
    }

    /// Whether the hold limit passed and the owner must restore now.
    pub fn expired(&self, now: Instant) -> bool {
        self.lease.expired(now)
    }

    /// Releases the lease and reads the aggregate target to prove it.
    pub fn restore(self) -> ReleaseOutcome {
        let device = self.device.clone();
        self.restore_with(|| super::pm_qos::read_effective(&device))
    }

    /// As `restore`, with the aggregate supplied by `aggregate` (the kernel stand-in in tests).
    pub fn restore_with(self, aggregate: impl FnOnce() -> io::Result<u32>) -> ReleaseOutcome {
        let (baseline, requested) = (self.baseline_us, self.lease.requested_us);
        drop(self.lease);
        match aggregate() {
            Ok(effective) if effective == baseline => ReleaseOutcome::Restored {
                effective_us: effective,
            },
            Ok(effective) if effective != requested => {
                ReleaseOutcome::ReleasedOtherRequesterActive {
                    effective_us: effective,
                }
            }
            _ => ReleaseOutcome::Unproven,
        }
    }
}

#[cfg(test)]
#[path = "pm_qos_transaction_tests.rs"]
mod tests;
