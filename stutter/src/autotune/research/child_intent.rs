//! The intent of one child study (PLAN section 1 nesting). A child reuses the normal journalled
//! `Driver` unchanged: it runs on a copy of the campaign's frozen intent whose attempt budget is the
//! study's own and which carries the study's digest, so its schedule, journal, dispatch contexts
//! and report are distinct from its siblings' and from the whole-campaign intent. Nothing else in
//! the frozen intent (identity, policy, tools, window, consent) may differ.

use super::{ResearchIntentV1, child_study::ChildStudy};

/// Derive the child's intent. The study's attempts must fill whole blocks across the tools.
pub fn child_intent(
    parent: &ResearchIntentV1,
    study: &ChildStudy,
) -> anyhow::Result<ResearchIntentV1> {
    anyhow::ensure!(
        parent.child_study_sha256.is_none(),
        "a child study cannot be nested in another"
    );
    let arms = u32::try_from(parent.tools.len())?;
    anyhow::ensure!(arms >= 2, "a comparison needs at least two eligible tools");
    anyhow::ensure!(
        study.min_attempts > 0 && study.min_attempts.is_multiple_of(arms),
        "the study's attempts do not fill whole blocks across the tools"
    );
    anyhow::ensure!(
        study.min_attempts <= parent.max_attempts,
        "the study needs more attempts than the campaign allows"
    );
    let mut child = parent.clone();
    child.max_attempts = study.min_attempts;
    child.child_study_sha256 = Some(study.study_sha256.clone());
    child.validate()?;
    Ok(child)
}

#[cfg(test)]
#[path = "child_intent_tests.rs"]
mod tests;
