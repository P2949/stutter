//! The schedule of an initialization study: not a comparison, so no balanced blocks and no minimum of
//! two arms. One attempt per listed arm, in that order, so a single cold arm is schedulable.

use super::{AttemptSlot, ProtonSource, ResearchIntentV1};

pub(super) fn slots(
    intent: &ResearchIntentV1,
    digest: &str,
    arms: &[ProtonSource],
) -> anyhow::Result<Vec<AttemptSlot>> {
    anyhow::ensure!(
        !arms.is_empty() && intent.max_attempts as usize == arms.len(),
        "an initialization study runs each listed arm exactly once"
    );
    let context_sha256 = stutter_core::hash::sha256_hex(
        format!(
            "{}:{}:{}",
            intent.source_sha256, intent.policy_sha256, intent.build_id
        )
        .as_bytes(),
    );
    Ok(arms
        .iter()
        .enumerate()
        .map(|(index, arm)| AttemptSlot {
            study_sha256: digest.to_owned(),
            context_sha256: context_sha256.clone(),
            index: index as u32,
            arm: *arm,
        })
        .collect())
}
