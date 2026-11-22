//! Which of an action study's scheduled attempts the action was actually applied on. An action study
//! compares attempts treated Off with attempts treated On over one Proton build, and eight completed
//! runs are not four valid treatment pairs unless the action was applied on every attempt treated
//! On. Each restoration receipt names the process the action ran against; this joins those receipts
//! to the sealed native completions (which name each attempt's game process) and to the study's
//! scheduled treatments. A receipt that belongs to no completed On attempt is an integrity failure;
//! an On attempt with no receipt means the comparison is of the controller's policy (intention to
//! treat), not of an applied action, and the two are never reported as the same experiment.

use super::{Treatment, child_verify::AttemptProcess};

/// The record an action study leaves when it ran and the controller never applied the action.
pub const ABSTENTION_RECORD: &str = "action-abstention.json";

/// What the receipts say of the On attempts that completed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Coverage {
    /// Completed attempts the schedule treated On.
    pub on_completed: u32,
    /// Of those, the attempts that carry at least one receipt for an applied action.
    pub on_applied: u32,
}

impl Coverage {
    /// Every completed On attempt (and at least one) had the action applied and restored: a
    /// comparison of an applied action.
    pub fn applied_on_every_treated_attempt(&self) -> bool {
        self.on_completed > 0 && self.on_applied == self.on_completed
    }
}

/// Join receipts to the study's completed attempts. `treatments` is the schedule by attempt slot.
pub fn attribute(
    receipts: &serde_json::Value,
    processes: &[AttemptProcess],
    treatments: &[Treatment],
) -> Result<Coverage, &'static str> {
    let is_on = |slot: u32| treatments.get(slot as usize) == Some(&Treatment::On);
    let mut applied = std::collections::BTreeSet::new();
    for item in receipts.as_array().map_or(&[][..], Vec::as_slice) {
        let process = processes.iter().find(|process| {
            item["pid"].as_u64() == Some(u64::from(process.pid.as_u32()))
                && item["start_ticks"].as_u64() == Some(process.start_ticks)
                && item["uid"].as_u64() == Some(u64::from(process.uid))
        });
        let Some(process) = process else {
            return Err("ReceiptForNoCompletedAttempt");
        };
        if !is_on(process.slot) {
            return Err("ReceiptOnAnUntreatedAttempt");
        }
        applied.insert(process.slot);
    }
    let on_completed = processes
        .iter()
        .filter(|process| is_on(process.slot))
        .count();
    Ok(Coverage {
        on_completed: u32::try_from(on_completed).unwrap_or(u32::MAX),
        on_applied: u32::try_from(applied.len()).unwrap_or(u32::MAX),
    })
}

#[cfg(test)]
#[path = "action_coverage_tests.rs"]
mod tests;
