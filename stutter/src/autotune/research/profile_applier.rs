//! Graphics-profile application inside a study-owned isolated prefix (PLAN section 6.1). The CPU
//! and GPU study profiles differ in the game's own settings, so a child study must be able to put a
//! frozen profile into the arm's `UserSettings.json`, prove by reading the file back that exactly
//! those settings changed, and restore the original bytes afterwards. The game's file stores each
//! option's `value`, and for list options also its `index`; the index of a value that was never
//! observed is not derivable, so such an edit is refused as `BlockedProfileMapping` and never
//! guessed. Nothing here touches Steam's own files or a prefix outside the study.

use std::{
    io::Write,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

use serde_json::Value;

const MAX_SETTINGS_BYTES: u64 = 4 * 1024 * 1024;

/// A value to put into one option.
#[derive(Clone, Debug, PartialEq)]
pub enum Setting {
    Bool(bool),
    Int(i64),
    /// A list option: the value text and its observed `index` (`None` = never observed).
    Choice {
        value: &'static str,
        index: Option<u32>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProfileEdit {
    pub group: &'static str,
    pub name: &'static str,
    pub to: Setting,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProfileSpec {
    pub name: &'static str,
    pub edits: Vec<ProfileEdit>,
}

/// Ray tracing off: only boolean options, all directly settable. The one profile edit set that
/// needs no list index (a lighter GPU workload at the user's own resolution).
pub fn ray_tracing_off() -> ProfileSpec {
    let off = |name| ProfileEdit {
        group: "/graphics/raytracing",
        name,
        to: Setting::Bool(false),
    };
    ProfileSpec {
        name: "ray_tracing_off",
        edits: vec![
            off("RayTracing"),
            off("RayTracedReflections"),
            off("RayTracedSunShadows"),
            off("RayTracedLocalShadows"),
            off("RayTracedPathTracing"),
        ],
    }
}

/// The provisional CPU-sensitive profile: 1080p raster, ray tracing off. Its resolution entry is a
/// list option whose index has not been observed, so applying it is `BlockedProfileMapping` until
/// that index is observed from a real file.
pub fn cpu_profile() -> ProfileSpec {
    let mut spec = ray_tracing_off();
    spec.name = "cpu_1080p_raster";
    spec.edits.push(ProfileEdit {
        group: "/video/display",
        name: "Resolution",
        to: Setting::Choice {
            value: "1920x1080",
            index: None,
        },
    });
    spec
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The settings file is not valid JSON of the expected shape.
    Malformed,
    UnknownSetting(String),
    TypeMismatch(String),
    /// A list value whose index was never observed.
    BlockedProfileMapping(String),
    /// The settings path is outside the study prefix, a symlink, foreign-owned or not a file.
    UnsafeFile,
    /// The game is running: its settings must not change under it.
    GameRunning,
    /// The file read back after writing does not equal the plan (or changed something else).
    ReadbackMismatch,
    /// The file no longer holds what was applied, or the restoration did not reproduce the bytes.
    RestoreMismatch,
    /// A snapshot of an earlier, unrestored application exists: recover it first.
    RecoveryPending,
    /// Putting the original bytes back could not be proven. The snapshot is kept; this is an
    /// integrity stop, not an ordinary failure.
    RestoreUnproven,
    Io,
}

/// One validated edit.
#[derive(Clone, Debug, PartialEq)]
struct Planned {
    group: &'static str,
    name: &'static str,
    to: Setting,
}

fn option_mut<'a>(root: &'a mut Value, group: &str, name: &str) -> Option<&'a mut Value> {
    root.get_mut("data")?
        .as_array_mut()?
        .iter_mut()
        .find(|entry| entry["group_name"] == group)?
        .get_mut("options")?
        .as_array_mut()?
        .iter_mut()
        .find(|option| option["name"] == name)
}

fn type_matches(option: &Value, to: &Setting) -> bool {
    matches!(
        (option["type"].as_str(), to),
        (Some("bool"), Setting::Bool(_))
            | (Some("int"), Setting::Int(_))
            | (
                Some("string_list" | "int_list" | "name_list"),
                Setting::Choice { .. }
            )
    )
}

/// Validate every edit against the file as it is now. Pure: nothing is written.
fn plan(settings: &[u8], spec: &ProfileSpec) -> Result<(Value, Vec<Planned>), Refusal> {
    let mut root: Value = serde_json::from_slice(settings).map_err(|_| Refusal::Malformed)?;
    if !root["data"].is_array() {
        return Err(Refusal::Malformed);
    }
    let mut planned = Vec::new();
    for edit in &spec.edits {
        let label = format!("{}/{}", edit.group, edit.name);
        let option = option_mut(&mut root, edit.group, edit.name)
            .ok_or_else(|| Refusal::UnknownSetting(label.clone()))?;
        if !type_matches(option, &edit.to) {
            return Err(Refusal::TypeMismatch(label));
        }
        if matches!(edit.to, Setting::Choice { index: None, .. }) {
            return Err(Refusal::BlockedProfileMapping(label));
        }
        planned.push(Planned {
            group: edit.group,
            name: edit.name,
            to: edit.to.clone(),
        });
    }
    Ok((root, planned))
}

fn write_edit(root: &mut Value, edit: &Planned) {
    if let Some(option) = option_mut(root, edit.group, edit.name) {
        match &edit.to {
            Setting::Bool(value) => option["value"] = Value::Bool(*value),
            Setting::Int(value) => option["value"] = Value::from(*value),
            Setting::Choice { value, index } => {
                option["value"] = Value::from(*value);
                option["index"] = index.map_or(Value::Null, Value::from);
            }
        }
    }
}

/// The edited document: every planned edit applied, everything else untouched.
fn edited(settings: &[u8], spec: &ProfileSpec) -> Result<(Vec<u8>, Vec<Planned>), Refusal> {
    let (mut root, planned) = plan(settings, spec)?;
    for edit in &planned {
        write_edit(&mut root, edit);
    }
    let bytes = serde_json::to_vec_pretty(&root).map_err(|_| Refusal::Malformed)?;
    Ok((bytes, planned))
}

/// The file now says exactly what was planned, and nothing else differs from `original`.
fn readback_matches(original: &[u8], now: &[u8], planned: &[Planned]) -> bool {
    let (Ok(mut expected), Ok(actual)) = (
        serde_json::from_slice::<Value>(original),
        serde_json::from_slice::<Value>(now),
    ) else {
        return false;
    };
    for edit in planned {
        write_edit(&mut expected, edit);
    }
    expected == actual
}

/// The only file the applier ever opens: a regular, user-owned, single-link `UserSettings.json`
/// inside the study's own directory, reached without following a symlink.
pub struct StudySettingsFile {
    path: PathBuf,
}

impl StudySettingsFile {
    pub fn new(study_root: &Path, path: PathBuf, owner_uid: u32) -> Result<Self, Refusal> {
        let canonical = std::fs::canonicalize(&path).map_err(|_| Refusal::UnsafeFile)?;
        let root = std::fs::canonicalize(study_root).map_err(|_| Refusal::UnsafeFile)?;
        let meta = std::fs::symlink_metadata(&path).map_err(|_| Refusal::UnsafeFile)?;
        if canonical != path
            || !canonical.starts_with(&root)
            || !meta.is_file()
            || meta.uid() != owner_uid
            || meta.nlink() != 1
            || meta.len() > MAX_SETTINGS_BYTES
            || path.file_name().and_then(|name| name.to_str()) != Some("UserSettings.json")
        {
            return Err(Refusal::UnsafeFile);
        }
        Ok(Self { path })
    }

    fn read(&self) -> Result<Vec<u8>, Refusal> {
        use std::io::Read;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&self.path)
            .map_err(|_| Refusal::Io)?;
        let mut bytes = Vec::new();
        file.take(MAX_SETTINGS_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| Refusal::Io)?;
        if bytes.len() as u64 > MAX_SETTINGS_BYTES {
            return Err(Refusal::UnsafeFile);
        }
        Ok(bytes)
    }

    /// Replace the file's bytes atomically (temporary sibling, fsync, rename).
    fn replace(&self, bytes: &[u8]) -> Result<(), Refusal> {
        let temporary = self.path.with_extension("stutter-tmp");
        let result = (|| {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            std::fs::rename(&temporary, &self.path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result.map_err(|_| Refusal::Io)
    }
}

impl StudySettingsFile {
    /// The durable copy of the original bytes, written before any edit and removed only after a
    /// proven restore. Its existence means the settings may still be edited.
    fn snapshot_path(&self) -> PathBuf {
        let mut name = self
            .path
            .file_name()
            .map(|n| n.to_os_string())
            .unwrap_or_default();
        name.push(".stutter-original");
        self.path.with_file_name(name)
    }

    /// Durably record the original bytes (private, new, fsynced with its directory).
    fn write_snapshot(&self, original: &[u8]) -> Result<(), Refusal> {
        let path = self.snapshot_path();
        let result = (|| {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)?;
            file.write_all(original)?;
            file.sync_all()?;
            std::fs::File::open(path.parent().unwrap_or(Path::new(".")))?.sync_all()
        })();
        match result {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                Err(Refusal::RecoveryPending)
            }
            Err(_) => {
                let _ = std::fs::remove_file(&path);
                Err(Refusal::Io)
            }
        }
    }

    /// The durable hash of the bytes this application writes, recorded before the edit: after a crash
    /// it tells the bytes we wrote from bytes something else wrote.
    fn marker_path(&self) -> PathBuf {
        let mut name = self
            .path
            .file_name()
            .map(|n| n.to_os_string())
            .unwrap_or_default();
        name.push(".stutter-applied");
        self.path.with_file_name(name)
    }

    fn write_marker(&self, applied: &[u8]) -> Result<(), Refusal> {
        let path = self.marker_path();
        let _ = std::fs::remove_file(&path);
        let result = (|| {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)?;
            file.write_all(stutter_core::hash::sha256_hex(applied).as_bytes())?;
            file.sync_all()?;
            std::fs::File::open(path.parent().unwrap_or(Path::new(".")))?.sync_all()
        })();
        result.map_err(|_| Refusal::Io)
    }

    fn read_marker(&self) -> Option<String> {
        std::fs::read_to_string(self.marker_path())
            .ok()
            .map(|text| text.trim().to_owned())
    }

    fn remove_snapshot(&self) -> Result<(), Refusal> {
        let path = self.snapshot_path();
        let _ = std::fs::remove_file(self.marker_path());
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            // Already removed by an earlier, proven restore: removing it again is a success.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err(Refusal::Io),
        }
        std::fs::File::open(path.parent().unwrap_or(Path::new(".")))
            .and_then(|dir| dir.sync_all())
            .map_err(|_| Refusal::Io)
    }

    fn read_snapshot(&self) -> Result<Option<Vec<u8>>, Refusal> {
        use std::io::Read;
        let path = self.snapshot_path();
        let file = match std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(Refusal::Io),
        };
        let meta = file.metadata().map_err(|_| Refusal::Io)?;
        if !meta.is_file() || meta.nlink() != 1 || meta.len() > MAX_SETTINGS_BYTES {
            return Err(Refusal::UnsafeFile);
        }
        let mut bytes = Vec::new();
        file.take(MAX_SETTINGS_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| Refusal::Io)?;
        Ok(Some(bytes))
    }
}

/// Proof that a profile was applied: what to restore and what the applied file must still be.
#[derive(Debug)]
pub struct AppliedProfile {
    pub profile: &'static str,
    original: Vec<u8>,
    applied_sha256: String,
}

impl AppliedProfile {
    pub fn original_sha256(&self) -> String {
        stutter_core::hash::sha256_hex(&self.original)
    }
}

/// Apply `spec` while the game is not running, verify by reading the file back, and return what is
/// needed to restore it. On any failure the original bytes are put back.
pub fn apply(
    file: &StudySettingsFile,
    spec: &ProfileSpec,
    game_running: bool,
) -> Result<AppliedProfile, Refusal> {
    if game_running {
        return Err(Refusal::GameRunning);
    }
    let original = file.read()?;
    let (bytes, planned) = edited(&original, spec)?;
    // The original is durable before the first edit: a crash from here on is recoverable.
    file.write_snapshot(&original)?;
    file.write_marker(&bytes)?;
    let replaced = file.replace(&bytes);
    let verified = replaced.and_then(|()| {
        file.read()
            .map(|now| readback_matches(&original, &now, &planned) && now == bytes)
    });
    if !matches!(verified, Ok(true)) {
        // Only a proven rollback ends the matter; otherwise the snapshot stays for recovery and the
        // caller is told the settings may still be edited.
        let proven = file.replace(&original).is_ok()
            && file.read().is_ok_and(|now| now == original)
            && file.remove_snapshot().is_ok();
        return Err(if proven {
            Refusal::ReadbackMismatch
        } else {
            Refusal::RestoreUnproven
        });
    }
    Ok(AppliedProfile {
        profile: spec.name,
        applied_sha256: stutter_core::hash::sha256_hex(&bytes),
        original,
    })
}

/// Put the original bytes back and prove it. A file that is no longer what was applied (the game
/// or something else rewrote it) is reported, never silently overwritten blind.
pub fn restore(file: &StudySettingsFile, applied: &AppliedProfile) -> Result<(), Refusal> {
    let current = file.read()?;
    let sha = stutter_core::hash::sha256_hex(&current);
    if sha != applied.applied_sha256 && sha != applied.original_sha256() {
        return Err(Refusal::RestoreMismatch);
    }
    if sha != applied.original_sha256() {
        file.replace(&applied.original)?;
    }
    if file.read()? == applied.original {
        file.remove_snapshot()
    } else {
        Err(Refusal::RestoreMismatch)
    }
}

/// What `recover` found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recovery {
    /// No snapshot: nothing was applied (or it was restored and proven).
    Nothing,
    /// A snapshot existed and the settings were already the original bytes.
    AlreadyOriginal,
    /// A snapshot existed; the settings were not the original and have been put back.
    Restored,
}

/// Whether a durable snapshot of an unrestored edit exists beside this settings path, whether or not
/// the settings file itself can be opened any longer.
pub fn pending_snapshot(settings: &Path) -> bool {
    let mut name = settings
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".stutter-original");
    std::fs::symlink_metadata(settings.with_file_name(name)).is_ok()
}

/// Whether an unrestored edit leaves any trace beside this settings path: the original's snapshot or
/// the applied-bytes marker. Used to check that nothing is still edited.
pub fn pending_edit(settings: &Path) -> bool {
    let beside = |suffix: &str| {
        let mut name = settings
            .file_name()
            .map(|n| n.to_os_string())
            .unwrap_or_default();
        name.push(suffix);
        settings.with_file_name(name)
    };
    // Only a clean "not found" proves there is no trace. Any other answer (a directory that cannot be
    // searched, an I/O error) leaves it unknown, and unknown is pending: nothing is claimed restored
    // on a question that could not be asked.
    [".stutter-original", ".stutter-applied"]
        .iter()
        .any(|suffix| match std::fs::symlink_metadata(beside(suffix)) {
            Ok(_) => true,
            Err(error) => error.kind() != std::io::ErrorKind::NotFound,
        })
}

/// After a crash or restart: if a durable snapshot exists the settings may still be edited, so put
/// the original bytes back, prove it, and only then remove the snapshot. Run before any study.
pub fn recover(file: &StudySettingsFile) -> Result<Recovery, Refusal> {
    let Some(original) = file.read_snapshot()? else {
        return Ok(Recovery::Nothing);
    };
    let current = file.read()?;
    let was_original = current == original;
    if !was_original {
        // Only bytes this application wrote may be put back; anything else was written by another
        // party since, and overwriting it from a stale snapshot would destroy its changes.
        let ours = file
            .read_marker()
            .is_some_and(|hash| hash == stutter_core::hash::sha256_hex(&current));
        if !ours {
            return Err(Refusal::RestoreMismatch);
        }
        file.replace(&original)?;
    }
    if file.read()? != original {
        return Err(Refusal::RestoreMismatch);
    }
    file.remove_snapshot()?;
    Ok(if was_original {
        Recovery::AlreadyOriginal
    } else {
        Recovery::Restored
    })
}

#[cfg(test)]
#[path = "profile_applier_tests.rs"]
mod tests;
