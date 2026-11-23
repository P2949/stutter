//! The real `ProfileHook`: applies a study's frozen graphics profile to every arm's own
//! `UserSettings.json` and restores them all. Application is all-or-nothing: if any arm cannot take
//! the profile, the arms already changed are put back before the failure is reported, so a study
//! never runs with arms on different settings.

use super::{
    child_study::Profile,
    profile_applier::{
        AppliedProfile, ProfileSpec, Refusal, StudySettingsFile, apply_guarded, cpu_profile,
        pending_edit, recover_guarded, restore_guarded,
    },
    study_executor::{ProfileFailure, ProfileHook},
};

/// The settings each study profile needs. The GPU profile's list indices have never been observed,
/// and the long profile changes no graphics settings.
pub fn production_spec(profile: Profile) -> Result<Option<ProfileSpec>, ProfileFailure> {
    match profile {
        Profile::Cpu => Ok(Some(cpu_profile())),
        Profile::Gpu => Ok(Some(super::profile_applier::gpu_profile())),
        Profile::Long => Ok(None),
    }
}

type MappingLoader = Box<
    dyn Fn(&[StudySettingsFile]) -> Result<super::profile_mappings::Mappings, ProfileFailure>
        + Send,
>;

pub struct FileProfileHook {
    files: Vec<StudySettingsFile>,
    /// Settings paths that could not be opened safely (no file yet, or an unsafe one). They cannot
    /// take a profile, but a durable snapshot beside one is still an unrestored edit.
    unavailable: Vec<std::path::PathBuf>,
    /// The arm each entry of `files` and of `unavailable` belongs to (parallel vectors, set together).
    file_arms: Vec<super::ProtonSource>,
    unavailable_arms: Vec<super::ProtonSource>,
    spec_for: fn(Profile) -> Result<Option<ProfileSpec>, ProfileFailure>,
    game_running: fn() -> bool,
    applied: Vec<(usize, AppliedProfile)>,
    /// Each edited settings file's digest as the profile left it.
    applied_digests: Vec<String>,
    /// Observed list mappings the profiles are resolved against before they are applied.
    mappings: super::profile_mappings::Mappings,
    mapping_loader: Option<MappingLoader>,
}

impl FileProfileHook {
    pub fn new(
        files: Vec<StudySettingsFile>,
        spec_for: fn(Profile) -> Result<Option<ProfileSpec>, ProfileFailure>,
        game_running: fn() -> bool,
    ) -> Self {
        Self {
            files,
            unavailable: Vec::new(),
            file_arms: Vec::new(),
            unavailable_arms: Vec::new(),
            spec_for,
            game_running,
            applied: Vec::new(),
            applied_digests: Vec::new(),
            mappings: super::profile_mappings::Mappings::default(),
            mapping_loader: None,
        }
    }

    /// The observed list mappings to resolve profiles against. Without them a profile that needs a
    /// list index is blocked as `BlockedProfileMapping`.
    pub fn with_mappings(mut self, mappings: super::profile_mappings::Mappings) -> Self {
        self.mappings = mappings;
        self
    }

    /// Load mappings from recovered settings immediately before applying the profile.
    pub fn with_mapping_loader(mut self, loader: MappingLoader) -> Self {
        self.mapping_loader = Some(loader);
        self
    }

    /// Record the arms whose settings file cannot be opened. Applying a profile is then refused (it
    /// is all-or-nothing), but recovery still inspects and restores every arm that can be opened and
    /// reports any pending snapshot it cannot inspect as unproven.
    /// Name the arm of each settings file and of each unavailable path, so a study that runs only some arms
    /// can apply a profile to just those.
    pub fn with_arm_map(
        mut self,
        file_arms: Vec<super::ProtonSource>,
        unavailable_arms: Vec<super::ProtonSource>,
    ) -> Self {
        self.file_arms = file_arms;
        self.unavailable_arms = unavailable_arms;
        self
    }

    pub fn with_unavailable(mut self, unavailable: Vec<std::path::PathBuf>) -> Self {
        self.unavailable = unavailable;
        self
    }
}

impl FileProfileHook {
    /// Apply `profile` to the settings of `selected` arms only (all of them when `None`). The unselected arms
    /// are neither edited nor required to be openable, and the digests recorded are in the campaign's own
    /// tool order over exactly the selected arms, which is the order the child's intent lists them in.
    fn apply_selected(
        &mut self,
        profile: Profile,
        selected: Option<&[super::ProtonSource]>,
    ) -> Result<bool, ProfileFailure> {
        let Some(spec) = (self.spec_for)(profile)? else {
            return Ok(false);
        };
        // Only the arms this study runs matter: an unusable unselected arm neither blocks nor is edited.
        let mapped = self.file_arms.len() == self.files.len()
            && self.unavailable_arms.len() == self.unavailable.len();
        // A requested subset needs the arm map to be honoured; without one the hook must not quietly apply every arm.
        if selected.is_some() && !mapped {
            return Err(ProfileFailure::Failed("ProfileArmMapMissing"));
        }
        let wanted = |arm: &super::ProtonSource| match (selected, mapped) {
            (Some(selected), true) => selected.contains(arm),
            _ => true,
        };
        let unavailable: Vec<&std::path::PathBuf> = if mapped {
            self.unavailable
                .iter()
                .zip(&self.unavailable_arms)
                .filter(|(_, arm)| wanted(arm))
                .map(|(path, _)| path)
                .collect()
        } else {
            self.unavailable.iter().collect()
        };
        if !unavailable.is_empty() {
            // A settings file that simply does not exist yet belongs to a prefix no game run has
            // initialized (cold). That is not a failure of the study: it cannot take a profile until
            // the prefix has been initialized, which needs its own supervised launch, and the study is
            // blocked with that reason. A file that exists but cannot be opened safely is a failure.
            let cold = unavailable.iter().all(|path| {
                matches!(
                    std::fs::symlink_metadata(path),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound
                )
            });
            return Err(if cold {
                ProfileFailure::Blocked("BlockedPrefixNotInitialized")
            } else {
                ProfileFailure::Failed("SettingsFileUnavailable")
            });
        }
        let running = (self.game_running)();
        if running {
            return Err(ProfileFailure::Failed("GameRunning"));
        }
        if let Some(loader) = &self.mapping_loader {
            let chosen: Vec<StudySettingsFile> = self
                .files
                .iter()
                .enumerate()
                .filter(|(index, _)| !mapped || wanted(&self.file_arms[*index]))
                .map(|(_, file)| file.clone())
                .collect();
            self.mappings = loader(&chosen)?;
        }
        let spec = self.mappings.resolve(spec);
        for (index, file) in self.files.iter().enumerate() {
            if mapped && !wanted(&self.file_arms[index]) {
                continue;
            }
            // Mapping discovery may take time. Recheck immediately before each edit, and never
            // roll back already-edited settings while a game is now running.
            if (self.game_running)() {
                return Err(if self.applied.is_empty() {
                    ProfileFailure::Failed("GameRunning")
                } else {
                    ProfileFailure::Unrestored("GameRunningDuringApply")
                });
            }
            match apply_guarded(file, &spec, &self.game_running) {
                Ok(applied) => self.applied.push((index, applied)),
                Err(refusal) => {
                    // Never restore earlier edits while a game may be using these settings. Keep
                    // the snapshots and ownership until independent settlement proves it is gone.
                    if !self.applied.is_empty() && (self.game_running)() {
                        return Err(ProfileFailure::Unrestored("GameRunningDuringRollback"));
                    }
                    // Put every arm already changed back before reporting; an unproven rollback
                    // is the worse failure and is what the caller must see.
                    let rolled_back = self.restore().is_ok();
                    return Err(if rolled_back {
                        failure(&refusal)
                    } else {
                        ProfileFailure::Unrestored("RollbackUnproven")
                    });
                }
            }
        }
        // What the games will run with: the digest of every file as the profile left it.
        // One digest per arm or none: an arm whose file cannot be read back is not silently dropped from
        // the record, it fails the application and puts the edits back.
        let digests: Option<Vec<String>> = self
            .applied
            .iter()
            .map(|(index, _)| {
                self.files[*index]
                    .read()
                    .ok()
                    .map(|bytes| stutter_core::hash::sha256_hex(&bytes))
            })
            .collect();
        let Some(digests) = digests else {
            return Err(if self.restore().is_ok() {
                ProfileFailure::Failed("SettingsDigestUnreadable")
            } else {
                ProfileFailure::Unrestored("RollbackUnproven")
            });
        };
        self.applied_digests = digests;
        Ok(true)
    }
}

fn failure(refusal: &Refusal) -> ProfileFailure {
    match refusal {
        Refusal::BlockedProfileMapping(_) => ProfileFailure::Blocked("BlockedProfileMapping"),
        Refusal::GameRunning => ProfileFailure::Failed("GameRunning"),
        Refusal::UnknownSetting(_) | Refusal::TypeMismatch(_) => {
            ProfileFailure::Failed("ProfileSettingMismatch")
        }
        Refusal::ReadbackMismatch => ProfileFailure::Failed("ReadbackMismatch"),
        // The settings may still be edited: an integrity stop, never an ordinary failure.
        Refusal::RestoreUnproven => ProfileFailure::Unrestored("RestoreUnproven"),
        Refusal::RecoveryPending => ProfileFailure::Unrestored("RecoveryPending"),
        _ => ProfileFailure::Failed("ProfileApplyFailed"),
    }
}

impl ProfileHook for FileProfileHook {
    fn apply(&mut self, profile: Profile) -> Result<bool, ProfileFailure> {
        self.apply_selected(profile, None)
    }

    fn apply_arms(
        &mut self,
        study: &super::child_study::ChildStudy,
        arms: Option<&[super::ProtonSource]>,
    ) -> Result<bool, ProfileFailure> {
        self.apply_selected(study.profile, arms)
    }

    fn applied_digests(&self) -> Vec<String> {
        self.applied_digests.clone()
    }

    fn recover(&mut self) -> Result<(), &'static str> {
        let mut proven = true;
        for file in &self.files {
            proven &= recover_guarded(file, &self.game_running).is_ok();
        }
        // An arm whose settings cannot be opened may still hold a snapshot of an unrestored edit:
        // that is never silently a no-op.
        for path in &self.unavailable {
            proven &= !pending_edit(path);
        }
        if proven {
            Ok(())
        } else {
            Err("RecoveryUnproven")
        }
    }

    fn restore(&mut self) -> Result<(), &'static str> {
        let mut proven = true;
        // A record whose restoration failed is kept (its durable snapshot is kept too), so the
        // restoration can be retried and is never forgotten.
        let mut failed = Vec::new();
        for (index, applied) in std::mem::take(&mut self.applied) {
            if restore_guarded(&self.files[index], &applied, &self.game_running).is_err() {
                proven = false;
                failed.push((index, applied));
            }
        }
        self.applied_digests.clear();
        self.applied = failed;
        if proven {
            Ok(())
        } else {
            Err("RestoreUnproven")
        }
    }
}

/// Whether a command line (NUL-separated arguments) is the game: one whole argument whose file name
/// is the Cyberpunk executable (either path separator), or Steam's own `AppId=1091500` launch
/// argument. A longer argument that merely mentions the name (a shell command, a search for it,
/// an editor opening a log) is not the game.
fn names_the_game(cmdline: &[u8]) -> bool {
    cmdline.split(|byte| *byte == 0).any(|argument| {
        let text = String::from_utf8_lossy(argument);
        text == "AppId=1091500"
            || text
                .rsplit(['/', '\\'])
                .next()
                .is_some_and(|name| name.eq_ignore_ascii_case("cyberpunk2077.exe"))
    })
}

/// The answer over the command lines of every process: the game is running if any process names
/// it, and an unreadable command line of a live process is unknown, which a settings edit treats
/// as running. A process that vanished mid-scan is not.
fn scan(lines: impl Iterator<Item = std::io::Result<Vec<u8>>>) -> bool {
    lines.into_iter().any(|line| match line {
        Ok(bytes) => names_the_game(&bytes),
        Err(error) => error.kind() != std::io::ErrorKind::NotFound,
    })
}

/// Whether any process of this machine is running the game; an unlistable /proc is unknown, which
/// is treated as the game running (the safe answer for a settings edit).
pub fn cyberpunk_running() -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return true;
    };
    scan(
        entries
            .flatten()
            .filter(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.bytes().all(|b| b.is_ascii_digit()))
            })
            .map(|entry| std::fs::read(entry.path().join("cmdline"))),
    )
}

#[cfg(test)]
#[path = "profile_hook_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "profile_production_tests.rs"]
mod production_tests;
