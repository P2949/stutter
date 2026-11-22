//! The owned-pressure control that must stand beside a justified "no action" result (PLAN G5). When the
//! controller correctly sees no eligible CPU pressure and abstains, that abstention only means something
//! if the same binary is shown to act when pressure does exist. The control is a bounded load the
//! campaign itself owns, applied while the real runtime observes it: enough scoped events, runnable
//! latency visibly worse under the load than before it, the supported action applied, and then restored.
//! This module is the contract and the independent reader; producing the receipt is a physical run.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// The file, beside the campaign's other private records, holding the receipt.
pub const FILE: &str = "pressure-control.json";
/// Fewer scoped events than this cannot show a latency difference.
pub const MIN_EVENTS: u64 = 1000;
/// A receipt older than this does not describe the deployed binary.
pub const MAX_AGE_SECONDS: u64 = 7 * 24 * 60 * 60;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PressureControlReceipt {
    pub schema: u32,
    pub binary_sha256: String,
    pub captured_unix: u64,
    /// The load was started and stopped by the campaign, not found on the machine.
    pub load_owned_by_campaign: bool,
    pub scoped_events: u64,
    pub baseline_runnable_p95_ms: f64,
    pub loaded_runnable_p95_ms: f64,
    pub action_applied: bool,
    pub restored_independently: bool,
}

/// Why the receipt cannot stand for the control, or `Ok`.
pub fn attested(
    receipt: &PressureControlReceipt,
    own_binary_sha256: &str,
    now: u64,
) -> Result<(), &'static str> {
    if receipt.schema != 1 {
        return Err("PressureControlSchemaUnsupported");
    }
    if receipt.binary_sha256 != own_binary_sha256 {
        return Err("PressureControlForAnotherBinary");
    }
    if receipt.captured_unix > now.saturating_add(60)
        || now.saturating_sub(receipt.captured_unix) > MAX_AGE_SECONDS
    {
        return Err("PressureControlNotFresh");
    }
    if !receipt.load_owned_by_campaign {
        return Err("PressureNotOwnedByCampaign");
    }
    if receipt.scoped_events < MIN_EVENTS {
        return Err("PressureControlTooFewEvents");
    }
    let (base, loaded) = (
        receipt.baseline_runnable_p95_ms,
        receipt.loaded_runnable_p95_ms,
    );
    if !(base.is_finite() && loaded.is_finite() && base > 0.0 && loaded > base) {
        return Err("PressureNotObserved");
    }
    if !receipt.action_applied {
        return Err("PressureControlActionNotApplied");
    }
    if !receipt.restored_independently {
        return Err("PressureControlNotRestored");
    }
    Ok(())
}

/// The control's status for a campaign directory: `NoPressureControl` when no receipt is there.
pub fn status(dir: &Path, own_binary_sha256: &str, now: u64) -> Result<(), &'static str> {
    // Opened through the directory without following a link, and measured on the open descriptor.
    let folder = std::fs::File::from(
        crate::syscall::open_dir_no_symlinks(dir).map_err(|_| "PressureControlUnusable")?,
    );
    let file = match crate::syscall::openat_nofollow(std::os::fd::AsFd::as_fd(&folder), FILE) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err("NoPressureControl");
        }
        Err(_) => return Err("PressureControlUnusable"),
    };
    let meta = file.metadata().map_err(|_| "PressureControlUnusable")?;
    if !meta.is_file() || meta.len() > 64 * 1024 {
        return Err("PressureControlUnusable");
    }
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut std::io::Read::take(file, 64 * 1024 + 1), &mut bytes)
        .map_err(|_| "PressureControlUnusable")?;
    let receipt: PressureControlReceipt =
        serde_json::from_slice(&bytes).map_err(|_| "PressureControlUnusable")?;
    attested(&receipt, own_binary_sha256, now)
}

#[cfg(test)]
#[path = "pressure_control_tests.rs"]
mod tests;
