//! A within-source treatment (PLAN section 1 nesting): one fixed Proton build measured with a factor
//! or an action off and on. The treatment is not a second Proton, so it is carried beside the
//! source, never as an arm: the schedule balances two *treatments* of one build, and the arm each
//! attempt records is that build either way. Every existing intent, journal, evidence file and
//! report is unchanged: the field is absent (not serialized) unless a treatment study sets it.

use serde::{Deserialize, Serialize};

use super::{ResearchAction, ResearchFactor, ResearchIntentV1, scheduler};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Treatment {
    Off,
    On,
}

/// What a treatment study switches on: one authorized factor or one authorized action.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TreatmentSpec {
    Factor(ResearchFactor),
    Action(ResearchAction),
}

/// The treatment of every scheduled attempt, in schedule order: a pure function of the intent (the
/// same seed as `build_schedule`), so a verifier regenerates it. `None` for an intent without a
/// treatment, whose attempts are the Proton arms themselves.
pub fn treatments_of(intent: &ResearchIntentV1) -> anyhow::Result<Option<Vec<Treatment>>> {
    if intent.treatment.is_none() {
        return Ok(None);
    }
    anyhow::ensure!(
        intent.tools.len() == 1,
        "a treatment study measures exactly one Proton build"
    );
    anyhow::ensure!(
        intent.max_attempts >= 2 && intent.max_attempts.is_multiple_of(2),
        "a treatment study needs whole off/on blocks"
    );
    let digest = intent.digest()?;
    let seed = u64::from_str_radix(&digest[..16], 16)?;
    let blocks = intent.max_attempts as usize / 2;
    let slots = scheduler::balanced(2, blocks, seed)
        .map_err(|error| anyhow::anyhow!("invalid schedule: {error:?}"))?;
    Ok(Some(
        slots
            .into_iter()
            .map(|slot| {
                if slot.arm == 0 {
                    Treatment::Off
                } else {
                    Treatment::On
                }
            })
            .collect(),
    ))
}

#[cfg(test)]
#[path = "treatment_tests.rs"]
mod tests;
