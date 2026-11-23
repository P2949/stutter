//! Per-factor backend proof (PLAN G7). Activation says a factor's environment variable reached the game, and the
//! game's own report (where it has one) says a selection was made; neither says the backend that implements the
//! factor was really in use. The proof is a witness of the running game process read from the kernel's own view of it,
//! never from what was asked for: the files its address space maps, the devices it holds open and its ancestor
//! processes. Each factor with a modelled backend names the one witness that must be present on every attempt
//! treated On and absent on every attempt treated Off. A factor with no modelled backend stays unproven; nothing is
//! guessed, and a generic "activated" receipt proves nothing here.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::{ResearchFactor, Treatment, child_verify::AttemptProcess};

/// The file, beside a feature study, holding one witness per completed attempt.
pub const FILE: &str = "feature-proof.json";
const MAX_ITEMS: usize = 8_192;

/// What the backend of a factor leaves visible in the game process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// A file mapped into the process (matched by its lowercase base name).
    Module(&'static str),
    /// A device node held open by the process.
    Device(&'static str),
    /// A process among the game's ancestors (matched by its lowercase command name).
    Ancestor(&'static str),
}

/// The modelled backend witness of a factor, or `None` when its backend is not modelled. The names were checked against
/// the builds installed on the research host: `winewayland.drv` (and its unix side `winewayland.so`) ships in GE-Proton and
/// the study prefix, `amdxcffx64.dll` ships in Proton Experimental's `contrib` and the prefix's `system32`, and
/// `/dev/ntsync` is the kernel's ntsync device node. The other factors are deliberately not modelled: the audio and input
/// backends act in Wine's services processes (`winepulse`, `winealsa`, `winebus`), not the game's, the low-latency layer
/// of `Vkd3dLowLatency` is a Vulkan implicit layer whose loading is not a mapped file of this build, and the CachyOS
/// tool directory here is a stub that cannot be inspected. Whether the game process really maps these at run time is
/// what a real run will show; a wrong name fails closed.
pub fn backend(factor: ResearchFactor) -> Option<Backend> {
    match factor {
        // Wine's Wayland display driver is a module of the game's own process.
        ResearchFactor::WineWayland => Some(Backend::Module("winewayland.drv")),
        // The NT synchronisation primitives are served by the ntsync device the Wine process holds open.
        ResearchFactor::NtSync => Some(Backend::Device("/dev/ntsync")),
        // The FSR4 runtime library the factor replaces the upscaler with.
        ResearchFactor::Fsr4 => Some(Backend::Module("amdxcffx64.dll")),
        // The game runs inside gamescope's session.
        ResearchFactor::Gamescope => Some(Backend::Ancestor("gamescope")),
        _ => None,
    }
}

/// What one process shows, bounded.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Witness {
    pub modules: BTreeSet<String>,
    pub devices: BTreeSet<String>,
    pub ancestors: BTreeSet<String>,
}

impl Witness {
    fn shows(&self, backend: Backend) -> bool {
        match backend {
            Backend::Module(name) => self.modules.contains(name),
            Backend::Device(path) => self.devices.contains(path),
            Backend::Ancestor(name) => self.ancestors.contains(name),
        }
    }
}

/// Reads the witness of a live process from /proc. `None` when the process cannot be read (gone, or not ours).
pub fn observe(pid: u32) -> Option<Witness> {
    let mut witness = Witness::default();
    for line in std::fs::read_to_string(format!("/proc/{pid}/maps"))
        .ok()?
        .lines()
    {
        if witness.modules.len() >= MAX_ITEMS {
            break;
        }
        // address perms offset dev inode path: the path is everything after the fifth field.
        if let Some(path) = line.split_whitespace().nth(5)
            && let Some(name) = path.rsplit('/').next()
        {
            witness.modules.insert(name.to_ascii_lowercase());
        }
    }
    for entry in std::fs::read_dir(format!("/proc/{pid}/fd")).ok()?.flatten() {
        if witness.devices.len() >= MAX_ITEMS {
            break;
        }
        if let Ok(target) = std::fs::read_link(entry.path())
            && let Some(text) = target.to_str()
            && text.starts_with("/dev/")
        {
            witness.devices.insert(text.to_owned());
        }
    }
    let mut current = pid;
    for _ in 0..16 {
        let stat = std::fs::read_to_string(format!("/proc/{current}/stat")).ok();
        // The parent is the second field after the parenthesised command name.
        let parent = stat
            .as_deref()
            .and_then(|stat| stat.rsplit_once(')'))
            .and_then(|(_, rest)| rest.split_whitespace().nth(1))
            .and_then(|field| field.parse::<u32>().ok());
        let Some(parent) = parent.filter(|parent| *parent > 1 && *parent != current) else {
            break;
        };
        if let Ok(comm) = std::fs::read_to_string(format!("/proc/{parent}/comm")) {
            witness.ancestors.insert(comm.trim().to_ascii_lowercase());
        }
        current = parent;
    }
    Some(witness)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    slot: u32,
    treatment: Treatment,
    pid: stutter_core::ids::Pid,
    start_ticks: u64,
    uid: u32,
    witness: Witness,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: u32,
    factor: ResearchFactor,
    attempts: Vec<Entry>,
}

/// The record a study writes: one witness per attempt, with the treatment and process it belongs to.
pub fn record_bytes(
    factor: ResearchFactor,
    attempts: &[(u32, Treatment, AttemptProcessKey, Witness)],
) -> Vec<u8> {
    let rows: Vec<serde_json::Value> = attempts
        .iter()
        .map(|(slot, treatment, process, witness)| {
            serde_json::json!({
                "slot": slot, "treatment": treatment, "pid": process.0,
                "start_ticks": process.1, "uid": process.2, "witness": witness,
            })
        })
        .collect();
    serde_json::to_vec(&serde_json::json!({ "schema": 1, "factor": factor, "attempts": rows }))
        .unwrap_or_default()
}

/// A process's pid, start time and uid.
pub type AttemptProcessKey = (stutter_core::ids::Pid, u64, u32);

/// One attempt's treatment, process, witness merged over the attempt and the unix second it was last read.
pub type AttemptWitness = (Treatment, AttemptProcessKey, Witness, u64);

/// Whether the factor's modelled backend was in use on every attempt treated On and on none treated Off, from
/// witnesses of the very processes the attempts recorded. Anything else, including a factor whose backend is not
/// modelled, is a typed reason and not proof.
pub fn verify(
    bytes: &[u8],
    factor: ResearchFactor,
    treatments: &[Treatment],
    processes: &[AttemptProcess],
) -> Result<(), &'static str> {
    let Some(backend) = backend(factor) else {
        return Err("FeatureBackendNotModelled");
    };
    let record: Record = serde_json::from_slice(bytes).map_err(|_| "FeatureProofUnreadable")?;
    if record.schema != 1 || record.factor != factor {
        return Err("FeatureProofForAnotherFactor");
    }
    let mut slots: Vec<u32> = record.attempts.iter().map(|entry| entry.slot).collect();
    slots.sort_unstable();
    let mut expected: Vec<u32> = processes.iter().map(|process| process.slot).collect();
    expected.sort_unstable();
    if slots != expected || expected.is_empty() {
        return Err("FeatureProofNotEveryAttempt");
    }
    let (mut on, mut off) = (0usize, 0usize);
    for entry in &record.attempts {
        let process = processes
            .iter()
            .find(|process| process.slot == entry.slot)
            .ok_or("FeatureProofNotEveryAttempt")?;
        if process.pid != entry.pid
            || process.start_ticks != entry.start_ticks
            || process.uid != entry.uid
        {
            return Err("FeatureProofForAnotherProcess");
        }
        if treatments.get(entry.slot as usize) != Some(&entry.treatment) {
            return Err("FeatureProofTreatmentMismatch");
        }
        match (entry.treatment, entry.witness.shows(backend)) {
            (Treatment::On, true) => on += 1,
            (Treatment::Off, false) => off += 1,
            (Treatment::On, false) => return Err("FeatureBackendAbsentWhereOn"),
            (Treatment::Off, true) => return Err("FeatureBackendPresentWhereOff"),
        }
    }
    (on > 0 && off > 0)
        .then_some(())
        .ok_or("FeatureProofNeedsBothTreatments")
}

#[cfg(test)]
#[path = "feature_proof_tests.rs"]
mod tests;
