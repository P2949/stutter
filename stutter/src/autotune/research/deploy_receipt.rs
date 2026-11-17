//! A read-only receipt of what is actually deployed (PLAN G8): which binary each Stutter unit runs,
//! whether that binary is the pinned one, and whether the units are active. A pin in a flake and a
//! built store object are not a running service; this reads the running service. It launches nothing,
//! changes nothing and is run by the administrator against the real `systemctl`. The verifier accepts
//! G8 only when a receipt names the very binary that is verifying the campaign and that binary is the
//! expected pin.

use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use serde::{Deserialize, Serialize};

/// A binary larger than this is not a Stutter release.
const MAX_BINARY_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitReceipt {
    pub unit: String,
    pub active_state: String,
    /// The store path the unit executes (from `ExecStart`).
    pub exec_path: String,
    pub binary_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployReceipt {
    pub schema: u32,
    pub captured_unix: u64,
    pub units: Vec<UnitReceipt>,
    /// The pin the administrator expects (SHA-256 of the release binary).
    pub expected_binary_sha256: String,
    /// Every unit runs a binary whose hash is the expected pin.
    pub binary_matches_pin: bool,
    /// Every unit is `active`.
    pub units_active: bool,
}

/// The value of one `Key=value` line of `systemctl show`.
fn property<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines()
        .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
}

/// The executable of the unit from its `ExecStart={ path=... ; argv[]=... }` property.
pub fn exec_path(show: &str) -> Option<String> {
    let value = property(show, "ExecStart")?;
    let path = value.split("path=").nth(1)?.split(" ;").next()?.trim();
    (!path.is_empty()).then(|| path.to_owned())
}

/// SHA-256 of the regular file at `path`, which must be an immutable store object.
pub fn binary_sha256(path: &Path, store_root: &Path) -> anyhow::Result<String> {
    anyhow::ensure!(
        path.starts_with(store_root),
        "the unit does not run an immutable store binary"
    );
    let canonical = std::fs::canonicalize(path)?;
    anyhow::ensure!(
        canonical.starts_with(std::fs::canonicalize(store_root)?),
        "the binary resolves outside the store"
    );
    let meta = std::fs::metadata(&canonical)?;
    anyhow::ensure!(
        meta.is_file() && meta.len() <= MAX_BINARY_BYTES,
        "the binary is not a bounded regular file"
    );
    let mut bytes = Vec::new();
    std::fs::File::open(&canonical)?
        .take(MAX_BINARY_BYTES)
        .read_to_end(&mut bytes)?;
    Ok(stutter_core::hash::sha256_hex(&bytes))
}

/// What one unit is, from its `systemctl show` text and the binary it names.
pub fn unit_receipt(unit: &str, show: &str, store_root: &Path) -> anyhow::Result<UnitReceipt> {
    let exec = exec_path(show).ok_or_else(|| anyhow::anyhow!("{unit} reports no ExecStart"))?;
    Ok(UnitReceipt {
        unit: unit.to_owned(),
        active_state: property(show, "ActiveState")
            .unwrap_or("unknown")
            .to_owned(),
        binary_sha256: binary_sha256(Path::new(&exec), store_root)?,
        exec_path: exec,
    })
}

/// Assemble the receipt from the units' receipts and the expected pin.
pub fn receipt(units: Vec<UnitReceipt>, expected: &str, now: u64) -> DeployReceipt {
    let binary_matches_pin =
        !units.is_empty() && units.iter().all(|unit| unit.binary_sha256 == expected);
    let units_active = !units.is_empty() && units.iter().all(|unit| unit.active_state == "active");
    DeployReceipt {
        schema: 1,
        captured_unix: now,
        units,
        expected_binary_sha256: expected.to_owned(),
        binary_matches_pin,
        units_active,
    }
}

/// The exact, root-owned `systemctl` to ask: the same trust the GPU-health journal reader requires.
pub fn trusted_systemctl(path: &Path) -> anyhow::Result<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    let canonical = std::fs::canonicalize(path)?;
    let meta = std::fs::metadata(&canonical)?;
    anyhow::ensure!(
        meta.is_file() && meta.uid() == 0 && meta.mode() & 0o022 == 0,
        "systemctl is not a root-owned, non-writable regular file"
    );
    Ok(canonical)
}

/// Ask `systemctl show` about one unit (read-only).
pub fn show(systemctl: &Path, unit: &str) -> anyhow::Result<String> {
    anyhow::ensure!(
        !unit.is_empty()
            && unit.len() <= 128
            && unit
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"@.-_".contains(&b)),
        "not a unit name"
    );
    let output = Command::new(systemctl)
        .args([
            "show",
            "--no-pager",
            "-p",
            "ActiveState",
            "-p",
            "ExecStart",
            unit,
        ])
        .stdin(Stdio::null())
        .output()?;
    anyhow::ensure!(output.status.success(), "systemctl show failed");
    anyhow::ensure!(output.stdout.len() <= 64 * 1024, "unexpectedly long output");
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The newest deploy receipt in a study directory, if one is there and readable.
pub fn read_latest(study: &Path) -> Option<DeployReceipt> {
    let mut best: Option<DeployReceipt> = None;
    for entry in std::fs::read_dir(study).ok()?.flatten() {
        let name = entry.file_name();
        let name = name.to_str()?;
        if !(name.starts_with("deploy-receipt-") && name.ends_with(".json")) {
            continue;
        }
        let Ok(bytes) = std::fs::read(entry.path()) else {
            continue;
        };
        if let Ok(candidate) = serde_json::from_slice::<DeployReceipt>(&bytes)
            && best
                .as_ref()
                .is_none_or(|current| candidate.captured_unix > current.captured_unix)
        {
            best = Some(candidate);
        }
    }
    best
}

/// G8's evidence: the newest receipt says every unit is active and runs the expected pin, and that
/// binary is the one performing this verification (a receipt about some other binary says nothing
/// about the system doing the checking).
pub fn deployment_attested(study: &Path, own_binary_sha256: &str) -> bool {
    read_latest(study).is_some_and(|receipt| {
        receipt.binary_matches_pin
            && receipt.units_active
            && receipt
                .units
                .iter()
                .all(|unit| unit.binary_sha256 == own_binary_sha256)
    })
}

/// The hash of the running executable (`/proc/self/exe`), `None` when it cannot be read.
pub fn own_binary_sha256() -> Option<String> {
    let mut bytes = Vec::new();
    std::fs::File::open("/proc/self/exe")
        .ok()?
        .take(MAX_BINARY_BYTES)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(stutter_core::hash::sha256_hex(&bytes))
}

#[cfg(test)]
#[path = "deploy_receipt_tests.rs"]
mod tests;
