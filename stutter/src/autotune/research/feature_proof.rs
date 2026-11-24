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
    /// Any one of several mapped files (the unix and the PE side of one Wine driver).
    Modules(&'static [&'static str]),
    /// The same file name mapped from one of two builds the tool ships: the kernel shows which build the process
    /// really mapped, by the content of the mapped file, against the hashes of the tool's own two variants (paths
    /// relative to the tool directory).
    Content {
        basename: &'static str,
        on: &'static str,
        off: &'static str,
    },
}

/// The tool-relative paths of the two builds a `Content` backend compares (on, off).
pub fn tool_builds(factor: ResearchFactor) -> Option<(&'static str, &'static str)> {
    match backend(factor) {
        Some(Backend::Content { on, off, .. }) => Some((on, off)),
        _ => None,
    }
}

/// Base names whose mapped files are hashed (those a `Content` backend compares).
pub fn watched(factor: ResearchFactor) -> Vec<&'static str> {
    match backend(factor) {
        Some(Backend::Content { basename, .. }) => vec![basename],
        // A library the factor loads is hashed too, so the version actually loaded is on record (a fallback build is a
        // different identity); the proof stands on its presence, the hash is the evidence beside it.
        Some(Backend::Module(name)) if name.ends_with(".dll") => vec![name],
        _ => Vec::new(),
    }
}

/// The modelled backend witness of a factor, or `None` when its backend is not modelled. Checked against the tools installed on
/// the research host: `winewayland.drv`/`.so` ship in GE-Proton and the CachyOS tool; `amdxcffx64.dll` ships in Proton
/// Experimental's `contrib`; `/dev/ntsync` is the kernel's device; the CachyOS tool's `proton` script removes the pipewire
/// configuration for `PROTON_USE_PIPEWIRE=0` and sets `WINE_AUDIO_DRIVER=pulse,alsa` (the tool ships `winepulse` and
/// `winepipewire`), and it copies `vkd3d-low-latency/d3d12core.dll` (sha256 78e63ad2…) instead of `vkd3d-proton/d3d12core.dll`
/// (b73d1c10…) into the prefix. Not modelled, each for a reason found in the same tools: `InputBackend` (`PROTON_USE_SDL`
/// only sets `PROTON_DISABLE_HIDRAW`/`PROTON_NO_STEAMINPUT`, which `winebus` reads in a services process, and both input buses
/// coexist so the baseline cannot be shown free of SDL), `FrameGeneration` (`MLFG_UPGRADE` appears nowhere in the Proton EM
/// tool: there is no artifact to witness), `AntiLag2` and `Hdr` (no source can switch them: `factor_env` reports no
/// environment control or no qualified output). A wrong name fails closed.
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
        // `PROTON_USE_PIPEWIRE=0` drops the pipewire driver and sets `WINE_AUDIO_DRIVER=pulse,alsa`: the pulse driver the
        // game process then loads is the witness (the pipewire build is the default and loads `winepipewire` instead).
        ResearchFactor::AudioBackend => Some(Backend::Modules(&["winepulse.so", "winepulse.drv"])),
        // The prefix receives `vkd3d-low-latency/d3d12core.dll` instead of `vkd3d-proton/d3d12core.dll`: two builds with
        // one name, told apart by the content of the file the process mapped.
        ResearchFactor::Vkd3dLowLatency => Some(Backend::Content {
            basename: "d3d12core.dll",
            on: "files/lib/wine/vkd3d-low-latency/x86_64-windows/d3d12core.dll",
            off: "files/lib/wine/vkd3d-proton/x86_64-windows/d3d12core.dll",
        }),
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
    /// SHA-256 of each mapped file whose base name was watched, by base name (one name can be mapped from several
    /// files).
    #[serde(default)]
    pub mapped: std::collections::BTreeMap<String, BTreeSet<String>>,
    /// The tool's own two variants of a content-compared file, as hashed from the tool directory the attempt used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_on_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_off_sha256: Option<String>,
    /// The scan of the process was complete: its maps, descriptors and ancestry were read whole, not cut at the bound.
    /// The absence of a module, device or ancestor proves nothing from a scan that was not.
    #[serde(default)]
    pub complete: bool,
}

impl Witness {
    /// The mapped file of that name is the `want` build and not the `other`.
    fn content_is(&self, basename: &str, want: Option<&str>, other: Option<&str>) -> bool {
        let (Some(want), Some(other)) = (want, other) else {
            return false;
        };
        self.mapped
            .get(basename)
            .is_some_and(|hashes| hashes.contains(want) && !hashes.contains(other))
    }

    fn shows(&self, backend: Backend) -> bool {
        match backend {
            Backend::Module(name) => self.modules.contains(name),
            Backend::Device(path) => self.devices.contains(path),
            Backend::Ancestor(name) => self.ancestors.contains(name),
            Backend::Modules(names) => names.iter().any(|name| self.modules.contains(*name)),
            Backend::Content { basename, .. } => self.content_is(
                basename,
                self.tool_on_sha256.as_deref(),
                self.tool_off_sha256.as_deref(),
            ),
        }
    }
}

/// SHA-256 of a file, refusing one over the bound (a mapped game DLL is far below it).
fn file_sha256(path: &str) -> Option<String> {
    const LIMIT: u64 = 64 * 1024 * 1024;
    let meta = std::fs::metadata(path).ok()?;
    (meta.is_file() && meta.len() <= LIMIT)
        .then(|| std::fs::read(path).ok())
        .flatten()
        .map(|bytes| stutter_core::hash::sha256_hex(&bytes))
}

/// SHA-256 of the file a process really mapped: read through that process's own root (its mount namespace, where a
/// containerised game's paths mean something), and only when that file has the inode the maps line names. A path that now
/// leads to another file (replaced since it was mapped) is not the mapped file and has no hash. The device is not compared:
/// on btrfs the maps line shows the superblock's device and `stat` the subvolume's, so the two never agree there.
fn mapped_file_sha256(pid: u32, path: &str, inode: &str) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let through_root = format!("/proc/{pid}/root{path}");
    let meta = std::fs::metadata(&through_root).ok()?;
    (inode.parse::<u64>().ok()? == meta.ino())
        .then(|| file_sha256(&through_root))
        .flatten()
}

/// Reads the witness of a live process from /proc. `None` when the process cannot be read (gone, or not ours).
pub fn observe(pid: u32, watch: &[&str]) -> Option<Witness> {
    let mut witness = Witness::default();
    let mut complete = true;
    for line in std::fs::read_to_string(format!("/proc/{pid}/maps"))
        .ok()?
        .lines()
    {
        if witness.modules.len() >= MAX_ITEMS {
            complete = false;
            break;
        }
        // address perms offset dev inode path: the path is everything after the fifth field.
        let fields: Vec<&str> = line.split_whitespace().collect();
        if let Some(&path) = fields.get(5)
            && let Some(name) = path.rsplit('/').next()
        {
            let name = name.to_ascii_lowercase();
            if watch.contains(&name.as_str())
                && !path.ends_with("(deleted)")
                && let Some(hash) = mapped_file_sha256(pid, path, fields[4])
            {
                witness.mapped.entry(name.clone()).or_default().insert(hash);
            }
            witness.modules.insert(name);
        }
    }
    for entry in std::fs::read_dir(format!("/proc/{pid}/fd")).ok()?.flatten() {
        if witness.devices.len() >= MAX_ITEMS {
            complete = false;
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
    witness.complete = complete;
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

/// Reads the witness of a process, hashing the mapped files whose base names are watched.
pub type WitnessReader = Box<dyn Fn(u32, &[&str]) -> Option<Witness> + Send>;

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
    let mut tool_hashes: Option<(&str, &str)> = None;
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
        if let Backend::Content { basename, .. } = backend {
            // The two builds must be known and distinct, the same on every attempt, and the process must show the
            // build its treatment asks for, not merely lack the other.
            let (Some(on_hash), Some(off_hash)) = (
                entry.witness.tool_on_sha256.as_deref(),
                entry.witness.tool_off_sha256.as_deref(),
            ) else {
                return Err("FeatureProofToolBuildsUnknown");
            };
            if on_hash == off_hash
                || tool_hashes.get_or_insert((on_hash, off_hash)) != &(on_hash, off_hash)
            {
                return Err("FeatureProofToolBuildsUnknown");
            }
            if entry.treatment == Treatment::Off
                && !entry
                    .witness
                    .content_is(basename, Some(off_hash), Some(on_hash))
            {
                return Err("FeatureProofOffBuildNotShown");
            }
        }
        // What the process did not show proves absence only from a scan that read it whole.
        if entry.treatment == Treatment::Off && !entry.witness.complete {
            return Err("FeatureProofScanIncomplete");
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
