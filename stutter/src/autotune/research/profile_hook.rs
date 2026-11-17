//! The real `ProfileHook`: applies a study's frozen graphics profile to every arm's own
//! `UserSettings.json` and restores them all. Application is all-or-nothing: if any arm cannot take
//! the profile, the arms already changed are put back before the failure is reported, so a study
//! never runs with arms on different settings.

use super::{
    child_study::Profile,
    profile_applier::{
        AppliedProfile, ProfileSpec, Refusal, StudySettingsFile, apply, cpu_profile,
        pending_snapshot, recover, restore,
    },
    study_executor::{ProfileFailure, ProfileHook},
};

/// The settings each study profile needs. The GPU profile's list indices have never been observed,
/// and the long profile changes no graphics settings.
pub fn production_spec(profile: Profile) -> Result<Option<ProfileSpec>, ProfileFailure> {
    match profile {
        Profile::Cpu => Ok(Some(cpu_profile())),
        Profile::Gpu => Err(ProfileFailure::Blocked("BlockedProfileMapping")),
        Profile::Long => Ok(None),
    }
}

pub struct FileProfileHook {
    files: Vec<StudySettingsFile>,
    /// Settings paths that could not be opened safely (no file yet, or an unsafe one). They cannot
    /// take a profile, but a durable snapshot beside one is still an unrestored edit.
    unavailable: Vec<std::path::PathBuf>,
    spec_for: fn(Profile) -> Result<Option<ProfileSpec>, ProfileFailure>,
    game_running: fn() -> bool,
    applied: Vec<(usize, AppliedProfile)>,
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
            spec_for,
            game_running,
            applied: Vec::new(),
        }
    }

    /// Record the arms whose settings file cannot be opened. Applying a profile is then refused (it
    /// is all-or-nothing), but recovery still inspects and restores every arm that can be opened and
    /// reports any pending snapshot it cannot inspect as unproven.
    pub fn with_unavailable(mut self, unavailable: Vec<std::path::PathBuf>) -> Self {
        self.unavailable = unavailable;
        self
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
        _ => ProfileFailure::Failed("ProfileApplyFailed"),
    }
}

impl ProfileHook for FileProfileHook {
    fn apply(&mut self, profile: Profile) -> Result<bool, ProfileFailure> {
        let Some(spec) = (self.spec_for)(profile)? else {
            return Ok(false);
        };
        if !self.unavailable.is_empty() {
            // A settings file that simply does not exist yet belongs to a prefix no game run has
            // initialized (cold). That is not a failure of the study: it cannot take a profile until
            // the prefix has been initialized, which needs its own supervised launch, and the study is
            // blocked with that reason. A file that exists but cannot be opened safely is a failure.
            let cold = self.unavailable.iter().all(|path| {
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
        for (index, file) in self.files.iter().enumerate() {
            match apply(file, &spec, running) {
                Ok(applied) => self.applied.push((index, applied)),
                Err(refusal) => {
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
        Ok(true)
    }

    fn recover(&mut self) -> Result<(), &'static str> {
        let mut proven = true;
        for file in &self.files {
            proven &= recover(file).is_ok();
        }
        // An arm whose settings cannot be opened may still hold a snapshot of an unrestored edit:
        // that is never silently a no-op.
        for path in &self.unavailable {
            proven &= !pending_snapshot(path);
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
            if restore(&self.files[index], &applied).is_err() {
                proven = false;
                failed.push((index, applied));
            }
        }
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
