//! The contract of a CP-LONG work unit (PLAN section 7): a repeatable, unattended, game-internal
//! long session in which the normal controller can observe, diagnose, act and settle. The native
//! benchmark is a short fixed scene; its FPS is not a long-session result and is never reported as
//! one. A study on the long profile therefore needs a work unit that satisfies this contract, and
//! when none is installed the study is decided `BlockedNoComparableLongWorkUnit` rather than run on
//! the benchmark. This module is the typed contract and the gate; the physical adapter that drives
//! the game through such a unit does not exist yet.

use super::child_study::{ChildStudy, Profile};

/// A long session shorter than this cannot show controller behaviour across phases.
pub const MIN_LONG_SESSION_SECONDS: u32 = 20 * 60;

/// A longer one exceeds what one attempt may hold the desktop for.
pub const MAX_LONG_SESSION_SECONDS: u32 = 60 * 60;

/// One repeatable long session, identified by what makes two runs of it comparable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LongWorkUnit {
    /// A stable name for the session (for example a save-and-route label).
    pub id: String,
    /// The planned length of one run, in seconds.
    pub duration_seconds: u32,
    /// SHA-256 (hex) over the session's definition: the save, the route and the in-game conditions
    /// that must be identical across runs for them to be comparable.
    pub definition_sha256: String,
}

impl LongWorkUnit {
    /// Whether the unit can stand for a comparable long session at all.
    pub fn validate(&self) -> Result<(), &'static str> {
        let hex = self.definition_sha256.len() == 64
            && self
                .definition_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        let named = !self.id.trim().is_empty() && self.id.len() <= 128;
        let sized =
            (MIN_LONG_SESSION_SECONDS..=MAX_LONG_SESSION_SECONDS).contains(&self.duration_seconds);
        if named && hex && sized {
            Ok(())
        } else {
            Err("BlockedInvalidLongWorkUnit")
        }
    }
}

/// Why a study on the long profile cannot run: no work unit is installed, or the installed one is
/// not a valid long session. Every other study is unaffected.
pub fn long_unit_blocker(study: &ChildStudy, unit: Option<&LongWorkUnit>) -> Option<&'static str> {
    if study.profile != Profile::Long {
        return None;
    }
    match unit {
        None => Some("BlockedNoComparableLongWorkUnit"),
        Some(unit) => unit.validate().err(),
    }
}

#[cfg(test)]
#[path = "long_session_tests.rs"]
mod tests;
