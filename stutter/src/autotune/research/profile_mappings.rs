//! Observed list mappings for the graphics profiles. The game's settings file stores a list option
//! as a `value` text and an `index`; the index of a value that was never seen is not derivable, so a
//! profile that needs it is blocked rather than guessed. This module is where such mappings are
//! observed and remembered: every `UserSettings.json` read yields the (value, index) pair of each
//! tracked option's current selection, pairs from different files merge, and a pair that contradicts
//! an earlier one (the same value at another index, or another value at the same index) refuses the
//! merge instead of overwriting. Mappings are kept as immutable, content-named files in the study
//! directory, so they survive restarts and can be audited; a profile is resolved against them, and
//! whatever is still unobserved stays `BlockedProfileMapping`, naming the missing option.

use std::{collections::BTreeMap, fs::File, os::fd::AsFd};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    profile_applier::{ProfileSpec, Setting},
    report_store::{read_private, write_new},
};

/// The list options the study profiles edit.
pub const TRACKED: [(&str, &str); 3] = [
    ("/video/display", "Resolution"),
    ("/graphics/presets", "ResolutionScaling"),
    ("/graphics/presets", "FrameGeneration"),
];

const PREFIX: &str = "profile-mappings-";
const MAX_FILE: usize = 64 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mappings {
    schema: u32,
    /// `"<group>/<name>"` -> value text -> observed index.
    choices: BTreeMap<String, BTreeMap<String, u32>>,
}

fn label(group: &str, name: &str) -> String {
    format!("{group}/{name}")
}

impl Mappings {
    pub fn is_empty(&self) -> bool {
        self.choices.values().all(BTreeMap::is_empty)
    }

    /// The pairs a settings file shows for the tracked options (their current selections).
    pub fn observe(settings: &[u8]) -> Option<Self> {
        let root: Value = serde_json::from_slice(settings).ok()?;
        let mut found = Self {
            schema: 1,
            choices: BTreeMap::new(),
        };
        for (group, name) in TRACKED {
            let option = root["data"]
                .as_array()?
                .iter()
                .find(|entry| entry["group_name"] == group)
                .and_then(|entry| entry["options"].as_array())
                .and_then(|options| options.iter().find(|option| option["name"] == name));
            let Some(option) = option else { continue };
            let list = matches!(
                option["type"].as_str(),
                Some("string_list" | "int_list" | "name_list")
            );
            let (Some(value), Some(index)) = (option["value"].as_str(), option["index"].as_u64())
            else {
                continue;
            };
            if list && !value.is_empty() && value.len() <= 128 && index < 4096 {
                found
                    .choices
                    .entry(label(group, name))
                    .or_default()
                    .insert(value.to_owned(), index as u32);
            }
        }
        Some(found)
    }

    /// Fold another set of observations in. A contradiction refuses the merge, naming the option.
    pub fn merge(&mut self, other: &Self) -> Result<(), String> {
        for (key, values) in &other.choices {
            let ours = self.choices.entry(key.clone()).or_default();
            for (value, index) in values {
                let same_value = ours.get(value).is_some_and(|known| known != index);
                let same_index = ours
                    .iter()
                    .any(|(known_value, known)| known == index && known_value != value);
                if same_value || same_index {
                    return Err(format!(
                        "{key}: {value} conflicts with an earlier observation"
                    ));
                }
                ours.insert(value.clone(), *index);
            }
        }
        self.schema = 1;
        Ok(())
    }

    fn index(&self, group: &str, name: &str, value: &str) -> Option<u32> {
        self.choices.get(&label(group, name))?.get(value).copied()
    }

    /// The spec with every list edit whose value has been observed given its index. An edit whose
    /// value has not stays `index: None`, which the applier refuses as `BlockedProfileMapping`.
    pub fn resolve(&self, mut spec: ProfileSpec) -> ProfileSpec {
        for edit in &mut spec.edits {
            if let Setting::Choice { value, index } = &mut edit.to
                && index.is_none()
            {
                *index = self.index(edit.group, edit.name, value);
            }
        }
        spec
    }

    /// Remember these observations beside the study as an immutable, content-named file. A file that
    /// is already there with the same name has the same content, so repeating is harmless.
    pub fn save(&self, dir: &File) -> anyhow::Result<()> {
        if self.is_empty() {
            return Ok(());
        }
        let bytes = serde_json::to_vec(self)?;
        let name = format!(
            "{PREFIX}{}.json",
            &stutter_core::hash::sha256_hex(&bytes)[..16]
        );
        match write_new(dir, &name, &bytes) {
            Ok(()) => Ok(()),
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|io| io.kind() == std::io::ErrorKind::AlreadyExists) =>
            {
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    /// Every remembered observation in the study directory, merged. An unreadable, oversized or
    /// contradictory file is an error: mappings that cannot be trusted are not used.
    pub fn load(dir: &File, uid: u32) -> anyhow::Result<Self> {
        let mut all = Self::default();
        let listing = std::fs::read_dir(format!(
            "/proc/self/fd/{}",
            std::os::fd::AsRawFd::as_raw_fd(&dir.as_fd())
        ))?;
        for entry in listing {
            let name = entry?.file_name().to_string_lossy().into_owned();
            if !(name.starts_with(PREFIX) && name.ends_with(".json")) {
                continue;
            }
            let bytes = read_private(dir, &name, uid)?;
            anyhow::ensure!(bytes.len() <= MAX_FILE, "{name} is too large");
            let one: Self = serde_json::from_slice(&bytes)?;
            anyhow::ensure!(one.schema == 1, "{name} has an unsupported schema");
            all.merge(&one).map_err(|reason| anyhow::anyhow!(reason))?;
        }
        Ok(all)
    }
}

#[cfg(test)]
#[path = "profile_mappings_tests.rs"]
mod tests;
