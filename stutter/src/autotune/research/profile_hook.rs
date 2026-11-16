//! The real `ProfileHook`: applies a study's frozen graphics profile to every arm's own
//! `UserSettings.json` and restores them all. Application is all-or-nothing: if any arm cannot take
//! the profile, the arms already changed are put back before the failure is reported, so a study
//! never runs with arms on different settings.

use super::{
    child_study::Profile,
    profile_applier::{
        AppliedProfile, ProfileSpec, Refusal, StudySettingsFile, apply, cpu_profile, recover,
        restore,
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
            spec_for,
            game_running,
            applied: Vec::new(),
        }
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

/// The profile hook when the arms' settings files cannot be opened safely (a cold prefix has none
/// yet, or one is unsafe): a study that needs a graphics profile fails with the reason, never
/// "applies" to nothing, and a study that needs none runs.
pub struct UnavailableProfiles(pub &'static str);

impl ProfileHook for UnavailableProfiles {
    fn apply(&mut self, profile: Profile) -> Result<bool, ProfileFailure> {
        match profile {
            Profile::Long => Ok(false),
            Profile::Cpu | Profile::Gpu => Err(ProfileFailure::Failed(self.0)),
        }
    }
    fn recover(&mut self) -> Result<(), &'static str> {
        Ok(())
    }
    fn restore(&mut self) -> Result<(), &'static str> {
        Ok(())
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
