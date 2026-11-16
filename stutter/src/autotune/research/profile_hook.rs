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

#[cfg(test)]
#[path = "profile_hook_tests.rs"]
mod tests;
