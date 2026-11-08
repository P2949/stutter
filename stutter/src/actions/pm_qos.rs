//! CPU latency requests through `/dev/cpu_dma_latency` (PM QoS), as a lease (package W11d).
//!
//! The kernel honours the request only while the file descriptor stays open and drops it the
//! moment the descriptor closes, including when the holder crashes. That makes a lease the right
//! shape: acquire, verify the aggregate target, hold under a deadline, release by dropping. The
//! effect is system-wide (it keeps every CPU out of deep idle states), so it is never a low-risk
//! default: a floor on the latency, a hold limit and power guards are enforced before the write.
//! This module performs a write only when handed a path, and nothing here chooses to run it.
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    path::Path,
    time::{Duration, Instant},
};

/// The binary request is one native-endian `u32` of microseconds.
const REQUEST_BYTES: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PmQosPolicy {
    /// The lowest latency that may be requested. Zero would keep every CPU polling.
    pub min_latency_us: u32,
    /// The highest request that still means anything; above it the request is pointless.
    pub max_latency_us: u32,
    /// Longest a lease may be held before it must be released.
    pub max_hold: Duration,
    pub require_ac: bool,
    pub forbid_thermal_throttling: bool,
}

impl Default for PmQosPolicy {
    fn default() -> Self {
        Self {
            min_latency_us: 10,
            max_latency_us: 10_000,
            max_hold: Duration::from_secs(60),
            require_ac: true,
            forbid_thermal_throttling: true,
        }
    }
}

/// What the host looks like right now, from the CPU power sensor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PowerContext {
    pub on_ac: Option<bool>,
    pub throttle_events_recent: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PmQosDenial {
    BelowFloor,
    AboveCeiling,
    /// Battery, or AC state unknown, when AC is required.
    NotOnAc,
    ThermalThrottling,
    /// Throttling state unknown while it is forbidden.
    ThrottlingUnknown,
}

impl PmQosPolicy {
    /// Whether a request of `latency_us` may be made in `context`. Unknown power facts fail closed.
    pub fn permit(&self, latency_us: u32, context: &PowerContext) -> Result<(), PmQosDenial> {
        if latency_us < self.min_latency_us {
            return Err(PmQosDenial::BelowFloor);
        }
        if latency_us > self.max_latency_us {
            return Err(PmQosDenial::AboveCeiling);
        }
        if self.require_ac && context.on_ac != Some(true) {
            return Err(PmQosDenial::NotOnAc);
        }
        if self.forbid_thermal_throttling {
            match context.throttle_events_recent {
                None => return Err(PmQosDenial::ThrottlingUnknown),
                Some(events) if events > 0 => return Err(PmQosDenial::ThermalThrottling),
                Some(_) => {}
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum LeaseError {
    Denied(PmQosDenial),
    Io(io::Error),
    /// The aggregate target read back is higher than the request: another requester wins or the
    /// write did not take effect.
    NotInEffect {
        requested: u32,
        effective: u32,
    },
}

impl From<io::Error> for LeaseError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// A held request. Dropping it closes the descriptor, which releases the request, and so does the
/// holder's death.
#[derive(Debug)]
pub struct PmQosLease {
    _held: File,
    pub requested_us: u32,
    acquired: Instant,
    max_hold: Duration,
}

impl PmQosLease {
    /// Opens `device`, writes the request, then reads the aggregate target to prove it is in
    /// effect. The request is released again if verification fails.
    pub fn acquire(
        device: &Path,
        latency_us: u32,
        policy: &PmQosPolicy,
        context: &PowerContext,
    ) -> Result<Self, LeaseError> {
        Self::acquire_verified(device, latency_us, policy, context, || {
            read_effective(device)
        })
    }

    /// As `acquire`, with the aggregate target supplied by `aggregate`: the real device read in
    /// production, a stand-in for a competing requester in tests.
    pub fn acquire_verified(
        device: &Path,
        latency_us: u32,
        policy: &PmQosPolicy,
        context: &PowerContext,
        aggregate: impl FnOnce() -> io::Result<u32>,
    ) -> Result<Self, LeaseError> {
        policy
            .permit(latency_us, context)
            .map_err(LeaseError::Denied)?;
        let mut held = OpenOptions::new().write(true).open(device)?;
        held.write_all(&latency_us.to_ne_bytes())?;
        let effective = aggregate()?;
        if effective > latency_us {
            // Dropping `held` here releases the request.
            return Err(LeaseError::NotInEffect {
                requested: latency_us,
                effective,
            });
        }
        Ok(Self {
            _held: held,
            requested_us: latency_us,
            acquired: Instant::now(),
            max_hold: policy.max_hold,
        })
    }

    /// Whether the lease has been held past its limit and must be released by its owner.
    pub fn expired(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.acquired) >= self.max_hold
    }
}

/// The aggregate latency target the kernel currently applies, as four bytes read from the device.
pub fn read_effective(device: &Path) -> io::Result<u32> {
    let mut bytes = [0u8; REQUEST_BYTES];
    File::open(device)?.read_exact(&mut bytes)?;
    Ok(u32::from_ne_bytes(bytes))
}

#[cfg(test)]
#[path = "pm_qos_tests.rs"]
mod tests;
