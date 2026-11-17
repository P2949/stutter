//! The hardware and kernel a campaign ran on (PLAN G0: owner, boot, GPU and session receipt; P2: wrong
//! GPU or graphics stack). The intent freezes the boot, the game and the Proton builds, but a result is
//! only comparable across the campaign if the kernel, the CPU topology and the GPU and its driver were
//! the same throughout. The production host records this read-only identity before any launch, and
//! verification takes it again and names the first part that differs. It reads `/proc` and `/sys` only
//! and records no serial numbers, paths or addresses.

use std::{
    fs::File,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use super::report_store::{read_private, write_new};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gpu {
    pub vendor: String,
    pub device: String,
    pub driver: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hardware {
    pub schema: u32,
    pub kernel_release: String,
    pub cpu_model: String,
    pub online_cpus: String,
    /// Whether simultaneous multithreading is active, and each logical CPU's sibling list.
    pub smt_active: Option<bool>,
    pub thread_siblings: Vec<String>,
    pub gpus: Vec<Gpu>,
}

fn text(path: &Path) -> Option<String> {
    let value = std::fs::read_to_string(path).ok()?;
    let value = value.trim();
    (!value.is_empty() && value.len() <= 256).then(|| value.to_owned())
}

/// Where the kernel's views are read from (the real `/proc` and `/sys`, or a fixture's).
pub struct Roots {
    pub proc: PathBuf,
    pub sys: PathBuf,
}

impl Roots {
    pub fn system() -> Self {
        Self {
            proc: "/proc".into(),
            sys: "/sys".into(),
        }
    }
}

fn cpu_model(proc: &Path) -> Option<String> {
    std::fs::read_to_string(proc.join("cpuinfo"))
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("model name"))
        .and_then(|rest| rest.split_once(':'))
        .map(|(_, name)| name.trim().chars().take(128).collect())
}

/// The sibling list of every logical CPU, in CPU order (the SMT topology).
fn siblings(sys: &Path) -> Vec<String> {
    let cpus = sys.join("devices/system/cpu");
    let mut found: Vec<(u32, String)> = std::fs::read_dir(&cpus)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let index = name.strip_prefix("cpu")?.parse::<u32>().ok()?;
            let list = text(&entry.path().join("topology/thread_siblings_list"))?;
            Some((index, list))
        })
        .collect();
    found.sort();
    found.into_iter().map(|(_, list)| list).collect()
}

/// Every DRM card (not its connectors): vendor and device id, and the bound kernel driver's name.
fn gpus(sys: &Path) -> Vec<Gpu> {
    let mut cards: Vec<(String, Gpu)> = std::fs::read_dir(sys.join("class/drm"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let digits = name.strip_prefix("card")?;
            if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            let device = entry.path().join("device");
            let driver = std::fs::read_link(device.join("driver"))
                .ok()?
                .file_name()?
                .to_string_lossy()
                .into_owned();
            Some((
                name,
                Gpu {
                    vendor: text(&device.join("vendor"))?,
                    device: text(&device.join("device"))?,
                    driver,
                },
            ))
        })
        .collect();
    cards.sort_by(|a, b| a.0.cmp(&b.0));
    cards.into_iter().map(|(_, gpu)| gpu).collect()
}

/// Read the host's hardware identity.
pub fn capture(roots: &Roots) -> Result<Hardware, &'static str> {
    let kernel_release =
        text(&roots.proc.join("sys/kernel/osrelease")).ok_or("HostHardwareUnreadable")?;
    let cpu_model = cpu_model(&roots.proc).ok_or("HostHardwareUnreadable")?;
    let online_cpus =
        text(&roots.sys.join("devices/system/cpu/online")).ok_or("HostHardwareUnreadable")?;
    let thread_siblings = siblings(&roots.sys);
    if thread_siblings.is_empty() {
        return Err("HostHardwareUnreadable");
    }
    Ok(Hardware {
        schema: 1,
        kernel_release,
        cpu_model,
        online_cpus,
        smt_active: text(&roots.sys.join("devices/system/cpu/smt/active")).map(|v| v == "1"),
        thread_siblings,
        gpus: gpus(&roots.sys),
    })
}

/// The first part of the hardware that is not the recorded one.
pub fn compare(before: &Hardware, after: &Hardware) -> Result<(), &'static str> {
    if before.schema != 1 || after.schema != 1 {
        return Err("HostHardwareSchemaUnsupported");
    }
    if before.kernel_release != after.kernel_release {
        return Err("HostKernelChanged");
    }
    if before.cpu_model != after.cpu_model
        || before.online_cpus != after.online_cpus
        || before.smt_active != after.smt_active
        || before.thread_siblings != after.thread_siblings
    {
        return Err("HostCpuTopologyChanged");
    }
    if before.gpus != after.gpus {
        return Err("HostGpuChanged");
    }
    Ok(())
}

/// Record the identity before any launch beside the study. An existing record is kept; one that cannot
/// be made is an error, so nothing launches on hardware that was never recorded.
pub fn record_baseline(
    roots: &Roots,
    study_dir: &File,
    uid: u32,
    campaign: &str,
) -> anyhow::Result<()> {
    let file = super::report_store::baseline_name("host-hardware", campaign)?;
    if let Ok(bytes) = read_private(study_dir, &file, uid) {
        serde_json::from_slice::<Hardware>(&bytes)?;
        return Ok(());
    }
    let hardware = capture(roots).map_err(|reason| anyhow::anyhow!(reason))?;
    write_new(study_dir, &file, &serde_json::to_vec_pretty(&hardware)?)
}

/// The hardware now against the recorded baseline, or why not (`HostHardwareNotRecorded`).
pub fn status(
    roots: &Roots,
    study_dir: &File,
    uid: u32,
    campaign: &str,
) -> Result<(), &'static str> {
    let file = super::report_store::baseline_name("host-hardware", campaign)
        .map_err(|_| "CampaignIdentityInvalid")?;
    let before: Hardware = read_private(study_dir, &file, uid)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .ok_or("HostHardwareNotRecorded")?;
    compare(&before, &capture(roots)?)
}

#[cfg(test)]
#[path = "host_hardware_tests.rs"]
mod tests;
