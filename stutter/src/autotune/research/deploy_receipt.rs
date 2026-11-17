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
    /// SHA-256 of that configured binary.
    pub binary_sha256: String,
    /// The unit's main process (`MainPID`); zero when it has none.
    #[serde(default)]
    pub main_pid: u32,
    /// SHA-256 of the executable that process is actually running (`/proc/<pid>/exe`): a configured
    /// path is not necessarily the binary currently executing.
    #[serde(default)]
    pub running_binary_sha256: Option<String>,
    /// The first argument of that process (`/proc/<pid>/cmdline`), readable without privilege: for a
    /// content-addressed store binary it names the very file the process was started from. It is
    /// what the process itself reports, so it is weaker than the executable hash above, which only a
    /// privileged reader can take.
    #[serde(default)]
    pub running_argv0: Option<String>,
    /// The process's start time in clock ticks (`/proc/<pid>/stat`), binding the receipt to one
    /// process so a recycled pid is not mistaken for it.
    #[serde(default)]
    pub start_ticks: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployReceipt {
    pub schema: u32,
    pub captured_unix: u64,
    /// The boot the receipt was taken in: a receipt from before a reboot says nothing about now.
    #[serde(default)]
    pub boot_id: Option<String>,
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

/// The hash of the executable a process is actually running (`<proc_root>/<pid>/exe`), `None` when
/// it cannot be read or is not a bounded regular file.
pub fn running_binary_sha256(pid: u32, proc_root: &Path) -> Option<String> {
    if pid == 0 {
        return None;
    }
    let file = std::fs::File::open(proc_root.join(pid.to_string()).join("exe")).ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.len() > MAX_BINARY_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_BINARY_BYTES).read_to_end(&mut bytes).ok()?;
    Some(stutter_core::hash::sha256_hex(&bytes))
}

/// The first argument of a process (`<proc_root>/<pid>/cmdline`), `None` when unreadable or empty.
pub fn running_argv0(pid: u32, proc_root: &Path) -> Option<String> {
    if pid == 0 {
        return None;
    }
    let mut bytes = Vec::new();
    std::fs::File::open(proc_root.join(pid.to_string()).join("cmdline"))
        .ok()?
        .take(8192)
        .read_to_end(&mut bytes)
        .ok()?;
    let first = bytes.split(|byte| *byte == 0).next()?;
    (!first.is_empty()).then(|| String::from_utf8_lossy(first).into_owned())
}

/// The process's start time in clock ticks: field 22 of `<proc_root>/<pid>/stat`, counted after the
/// parenthesised command name (which may itself contain spaces or parentheses).
pub fn process_start_ticks(pid: u32, proc_root: &Path) -> Option<u64> {
    if pid == 0 {
        return None;
    }
    let mut text = String::new();
    std::fs::File::open(proc_root.join(pid.to_string()).join("stat"))
        .ok()?
        .take(8192)
        .read_to_string(&mut text)
        .ok()?;
    // After the final ')' the fields start at 3 (state); starttime is the 22nd, so the 20th here.
    text.rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(19)?
        .parse()
        .ok()
}

/// What one unit is, from its `systemctl show` text, the binary it names and the process it runs.
/// `proc_root` is `/proc` (a directory of per-process entries) outside tests.
pub fn unit_receipt(
    unit: &str,
    show: &str,
    store_root: &Path,
    proc_root: &Path,
) -> anyhow::Result<UnitReceipt> {
    let exec = exec_path(show).ok_or_else(|| anyhow::anyhow!("{unit} reports no ExecStart"))?;
    let main_pid = property(show, "MainPID")
        .and_then(|value| value.trim().parse::<u32>().ok())
        .unwrap_or(0);
    Ok(UnitReceipt {
        unit: unit.to_owned(),
        active_state: property(show, "ActiveState")
            .unwrap_or("unknown")
            .to_owned(),
        binary_sha256: binary_sha256(Path::new(&exec), store_root)?,
        exec_path: exec,
        main_pid,
        running_binary_sha256: running_binary_sha256(main_pid, proc_root),
        running_argv0: running_argv0(main_pid, proc_root),
        start_ticks: process_start_ticks(main_pid, proc_root),
    })
}

/// A unit counts as running the binary only when the configured binary hashes to it, it has a live,
/// identified main process, and that process is running it. The strongest evidence is the running
/// executable's own hash; where an unprivileged reader cannot take it (the service processes are not
/// readable by the user), the process must at least report having been started from the unit's own
/// immutable store path. A readable executable that differs from the pin always refuses.
fn unit_runs(unit: &UnitReceipt, binary: &str) -> bool {
    let process_is_it = match unit.running_binary_sha256.as_deref() {
        Some(running) => running == binary,
        None => unit.running_argv0.as_deref() == Some(unit.exec_path.as_str()),
    };
    unit.binary_sha256 == binary
        && unit.main_pid != 0
        && unit.start_ticks.is_some()
        && process_is_it
}

/// Assemble the receipt from the units' receipts and the expected pin.
pub fn receipt(
    units: Vec<UnitReceipt>,
    expected: &str,
    now: u64,
    boot_id: Option<String>,
) -> DeployReceipt {
    let binary_matches_pin =
        !units.is_empty() && units.iter().all(|unit| unit_runs(unit, expected));
    let units_active = !units.is_empty() && units.iter().all(|unit| unit.active_state == "active");
    DeployReceipt {
        schema: 2,
        captured_unix: now,
        boot_id,
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
            "-p",
            "MainPID",
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

/// The units a deployed Stutter must have, each exactly once (by name before the user instance).
pub const REQUIRED_UNIT_PREFIXES: [&str; 2] =
    ["stutter-autotune-bounded@", "stutter-privileged-worker@"];

/// A receipt older than this no longer says what is deployed now.
pub const MAX_RECEIPT_AGE_SECONDS: u64 = 24 * 60 * 60;

/// Why a receipt does not attest the deployment, or `Ok` when it does. Nothing here trusts the
/// receipt's stored summary booleans: everything is recomputed from its units, and the receipt must
/// be about this very binary, in this boot, taken recently, covering every required unit once.
/// What the receipt must be bound to besides time and boot: the campaign being verified. `revision` is the
/// Git revision embedded in the verifying binary and `campaign_source_sha256` the source identity the
/// campaign froze (a hash of the revision and binary of the build that submitted it).
pub struct CampaignBinding<'a> {
    pub revision: &'a str,
    pub campaign_source_sha256: &'a str,
}

impl<'a> CampaignBinding<'a> {
    /// The binding for a campaign that froze `campaign_source_sha256`, verified by this very build.
    pub fn for_this_build(campaign_source_sha256: &'a str) -> Self {
        Self {
            revision: env!("STUTTER_GIT_REV"),
            campaign_source_sha256,
        }
    }
}

pub fn attested(
    receipt: &DeployReceipt,
    own_binary_sha256: &str,
    current_boot: Option<&str>,
    now: u64,
    campaign: Option<CampaignBinding<'_>>,
) -> Result<(), &'static str> {
    if receipt.schema < 2 {
        return Err("ReceiptSchemaTooOld");
    }
    if receipt.expected_binary_sha256 != own_binary_sha256 {
        return Err("ReceiptPinIsNotThisBinary");
    }
    for prefix in REQUIRED_UNIT_PREFIXES {
        let matching = receipt
            .units
            .iter()
            .filter(|unit| unit.unit.starts_with(prefix))
            .count();
        match matching {
            0 => return Err("RequiredUnitMissing"),
            1 => {}
            _ => return Err("RequiredUnitDuplicated"),
        }
    }
    if receipt.units.len() != REQUIRED_UNIT_PREFIXES.len() {
        return Err("UnexpectedUnit");
    }
    for unit in &receipt.units {
        if unit.active_state != "active" {
            return Err("UnitNotActive");
        }
        if !unit_runs(unit, own_binary_sha256) {
            return Err("UnitNotRunningThisBinary");
        }
    }
    match (receipt.boot_id.as_deref(), current_boot) {
        (Some(taken), Some(current)) if taken == current => {}
        _ => return Err("ReceiptFromAnotherBoot"),
    }
    if receipt.captured_unix > now.saturating_add(60)
        || now.saturating_sub(receipt.captured_unix) > MAX_RECEIPT_AGE_SECONDS
    {
        return Err("ReceiptNotFresh");
    }
    // The deployed binary must be the one that froze the campaign being verified: its identity,
    // recomputed from this binary's revision and the pinned hash, is the campaign's frozen source.
    let Some(campaign) = campaign else {
        return Err("ReceiptNotBoundToACampaign");
    };
    match super::host::source_identity_of(campaign.revision, own_binary_sha256) {
        Ok(identity) if identity == campaign.campaign_source_sha256 => Ok(()),
        _ => Err("DeployedBinaryIsNotTheCampaignsBuild"),
    }
}

/// G8's evidence: the newest receipt attests the deployment of the very binary doing this
/// verification (see `attested`).
pub fn deployment_attested(
    study: &Path,
    own_binary_sha256: &str,
    current_boot: Option<&str>,
    now: u64,
    campaign: Option<CampaignBinding<'_>>,
) -> bool {
    read_latest(study).is_some_and(|receipt| {
        attested(&receipt, own_binary_sha256, current_boot, now, campaign).is_ok()
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
