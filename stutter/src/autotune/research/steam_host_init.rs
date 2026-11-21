//! The initialization purpose of the Steam attempt host: the attempt launches the game only so that it
//! creates its Wine prefix and settings. Nothing is measured; the game is stopped (and its process tree
//! proven gone through the port's own settlement) once the prefix holds what initialization creates, or
//! it is simply allowed to end. A result or its absence is not evidence here, and the placeholder
//! evidence carries no native completion.

use std::time::{Duration, Instant};

use super::{AttemptEvidenceV1, GameState, Progress, SteamAttemptHost, SteamPort};

/// The game's settings file, relative to the arm's compat directory (created once the game reaches
/// its menu; the screens' profile edits it).
pub(super) const SETTINGS_RELATIVE: &str =
    "pfx/drive_c/users/steamuser/AppData/Local/CD Projekt Red/Cyberpunk 2077/UserSettings.json";
/// The prefix must have held its registry and the settings file this long before the game is stopped,
/// so the files are complete.
pub(super) const INIT_SETTLE: Duration = Duration::from_secs(15);

impl<P: SteamPort> SteamAttemptHost<P> {
    /// Whether the current arm's prefix has the registry and the settings file.
    fn prefix_ready(&self) -> bool {
        let Some(arm) = self.current_arm else {
            return false;
        };
        let compat = self.port.study_compat_data().join(arm.dir_name());
        super::super::prefix_identity::observe(&compat).class
            == super::super::prefix_identity::PrefixClass::Warm
            && std::fs::symlink_metadata(compat.join(SETTINGS_RELATIVE))
                .is_ok_and(|meta| meta.is_file())
    }

    fn stop_into(&mut self, index: u32) -> Progress {
        if self.port.stop().is_err() {
            return Progress::Failed {
                class: super::super::driver::FailureClass::OwnershipUnresolved,
                code: "SettlementUnverified".into(),
            };
        }
        self.current = None;
        self.current_arm = None;
        self.warm_since = None;
        Progress::Finished {
            evidence: AttemptEvidenceV1 {
                long_session: None,
                schema_version: 1,
                slot_index: index,
                // No measurement exists; the record only has to be a valid, positive placeholder.
                average_fps: 1.0,
                native_completion: None,
            },
        }
    }

    /// One poll of an initialization attempt. `Err(state)` hands any state that voids or interrupts the
    /// launch back to the ordinary handling (which stops and fails the attempt).
    pub(super) fn init_poll(
        &mut self,
        index: u32,
        state: GameState,
    ) -> Result<Progress, GameState> {
        match state {
            GameState::Starting | GameState::Running => {
                if self.prefix_ready() {
                    let since = *self.warm_since.get_or_insert_with(Instant::now);
                    if since.elapsed() >= self.init_settle {
                        return Ok(self.stop_into(index));
                    }
                } else {
                    self.warm_since = None;
                }
                Ok(Progress::Running)
            }
            // The game ending by itself is also a way for initialization to finish.
            GameState::Result { .. }
            | GameState::ExitedWithoutResult
            | GameState::InvalidNativeResult => Ok(self.stop_into(index)),
            other => Err(other),
        }
    }
}

#[cfg(test)]
#[path = "steam_host_init_tests.rs"]
mod tests;
