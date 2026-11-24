//! The owned-pressure control that must stand beside a justified "no action" result (PLAN G5). When the
//! controller correctly sees no eligible CPU pressure and abstains, that abstention only means something
//! if the same binary is shown to act when pressure does exist. The control is a bounded load the
//! campaign itself owns, applied while the real runtime observes it: enough scoped events, runnable
//! latency visibly worse under the load than before it, the supported action applied, and then restored.
//! This module is the contract and the independent reader: the receipt is rebuilt from the control's raw record
//! (`pressure_evidence`); running the control is a physical run.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// The campaign-owned bounded load itself.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "staged: the campaign starts the load when it runs the control, which waits for the incident to clear"
    )
)]
#[path = "owned_load.rs"]
pub(crate) mod owned_load;
/// The raw record of the control and the receipt rebuilt from it.
#[path = "pressure_evidence.rs"]
pub(crate) mod pressure_evidence;
/// The control's order of events, halt checks and cleanup, apart from the world it runs in.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "staged: a real stage (the stand-in target and the runtime's own measurement) is supplied when the incident clears"
    )
)]
#[path = "pressure_runner.rs"]
pub(crate) mod pressure_runner;

pub use owned_load::LoadRecord;
pub use pressure_evidence::{EVIDENCE_FILE, PressureControlEvidence, reconstruct};

/// The file, beside the campaign's other private records, holding the receipt.
pub const FILE: &str = "pressure-control.json";
/// The largest control file read.
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
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

/// One file of the campaign directory, opened through it without following a link and measured on the open descriptor:
/// `None` when it is not there, and an error when it is not a small regular file.
fn read_in(folder: &std::fs::File, name: &str) -> Result<Option<Vec<u8>>, &'static str> {
    let file = match crate::syscall::openat_nofollow(std::os::fd::AsFd::as_fd(folder), name) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("PressureControlUnusable"),
    };
    let meta = file.metadata().map_err(|_| "PressureControlUnusable")?;
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        return Err("PressureControlUnusable");
    }
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(
        &mut std::io::Read::take(file, MAX_FILE_BYTES + 1),
        &mut bytes,
    )
    .map_err(|_| "PressureControlUnusable")?;
    Ok(Some(bytes))
}

/// The control's status for a campaign directory: `NoPressureControl` when its raw record is not there. The receipt is
/// derived from that record (the load's lifecycle, the populations before and under it, the gate's receipt and the
/// worker's export), never read as a claim; a receipt written beside it must say exactly what the record supports.
pub fn status(dir: &Path, own_binary_sha256: &str, now: u64) -> Result<(), &'static str> {
    let folder = std::fs::File::from(
        crate::syscall::open_dir_no_symlinks(dir).map_err(|_| "PressureControlUnusable")?,
    );
    let Some(bytes) = read_in(&folder, EVIDENCE_FILE)? else {
        return Err("NoPressureControl");
    };
    let evidence: PressureControlEvidence =
        serde_json::from_slice(&bytes).map_err(|_| "PressureControlUnusable")?;
    let derived = reconstruct(&evidence)?;
    if let Some(stated) = read_in(&folder, FILE)? {
        let stated: PressureControlReceipt =
            serde_json::from_slice(&stated).map_err(|_| "PressureControlUnusable")?;
        if stated != derived {
            return Err("PressureControlNotFromEvidence");
        }
    }
    attested(&derived, own_binary_sha256, now)
}

#[cfg(test)]
#[path = "pressure_control_tests.rs"]
mod tests;
