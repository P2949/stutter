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

/// Whether an intent's attempt time limits are a valid closed contract. A long window is authorized
/// on its own: when set it is a bounded long session that fits the active budget, and only a child
/// study (never the whole campaign) may run attempts of that length. Every other attempt keeps the
/// short benchmark limit.
pub fn attempt_limits_valid(intent: &super::ResearchIntentV1) -> bool {
    let window_ok = intent.long_attempt_seconds.is_none_or(|seconds| {
        (MIN_LONG_SESSION_SECONDS..=MAX_LONG_SESSION_SECONDS).contains(&seconds)
            && seconds <= intent.max_active_seconds
    });
    let attempt_ok = (1..=720).contains(&intent.max_attempt_seconds)
        || (intent.child_study_sha256.is_some()
            && intent.long_attempt_seconds == Some(intent.max_attempt_seconds));
    window_ok && attempt_ok && intent.max_attempt_seconds <= intent.max_active_seconds
}

/// Why a study on the long profile cannot run: no work unit is installed, or the installed one is
/// not a valid long session. Every other study is unaffected.
pub fn long_unit_blocker(
    study: &ChildStudy,
    unit: Option<&LongWorkUnit>,
    authorized_seconds: Option<u32>,
) -> Option<&'static str> {
    if study.profile != Profile::Long {
        return None;
    }
    let Some(unit) = unit else {
        return Some("BlockedNoComparableLongWorkUnit");
    };
    if let Err(reason) = unit.validate() {
        return Some(reason);
    }
    // The session must fit the separately authorized long window, which is not the short benchmark
    // limit: a work unit the driver would terminate before it ends cannot run.
    match authorized_seconds {
        Some(window) if unit.duration_seconds <= window => None,
        Some(_) => Some("BlockedLongWorkUnitExceedsAuthorizedWindow"),
        None => Some("BlockedNoLongAttemptAuthorization"),
    }
}

#[cfg(test)]
#[path = "long_session_tests.rs"]
mod tests;
