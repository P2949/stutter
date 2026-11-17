//! The intent of one child study (PLAN section 1 nesting). A child reuses the normal journalled
//! `Driver` unchanged: it runs on a copy of the campaign's frozen intent whose tools are the study's
//! own arm subset, whose attempt budget is the study's own, and which carries the study's digest,
//! so its schedule, journal, dispatch contexts and report are distinct from its siblings' and from
//! the whole-campaign intent. Nothing else in the frozen intent (identity, policy, window, consent)
//! may differ, and the arms must be tools the campaign already authorized.

use super::{
    ProtonSource, ResearchIntentV1,
    child_study::{ChildStudy, Stage, StudyArms},
    pilot_rule::FrozenConfirmationRule,
};

/// Derive the child's intent over `arms`: every tool of the campaign for a screen, exactly two of
/// its tools for a confirmation, feature or action study. The study's attempts must fill whole
/// blocks across those arms.
pub fn child_intent(
    parent: &ResearchIntentV1,
    study: &ChildStudy,
    arms: &[ProtonSource],
    rule: Option<&FrozenConfirmationRule>,
) -> anyhow::Result<ResearchIntentV1> {
    anyhow::ensure!(
        parent.child_study_sha256.is_none(),
        "a child study cannot be nested in another"
    );
    // The child's tools are the selected arms, in the campaign's own order and with its frozen
    // identities; nothing outside the authorization can be introduced.
    let tools: Vec<_> = parent
        .tools
        .iter()
        .filter(|tool| arms.contains(&tool.source))
        .cloned()
        .collect();
    let minimum = if study.arms() == StudyArms::Single {
        1
    } else {
        2
    };
    anyhow::ensure!(
        tools.len() == arms.len() && tools.len() >= minimum,
        "the arms are not distinct tools of this campaign"
    );
    match study.arms() {
        StudyArms::All => anyhow::ensure!(
            tools.len() == parent.tools.len(),
            "a screen runs across every authorized tool"
        ),
        StudyArms::Pair => {
            anyhow::ensure!(tools.len() == 2, "this study compares exactly two arms")
        }
        StudyArms::Single => anyhow::ensure!(
            tools.len() == 1,
            "a treatment study measures exactly one Proton build"
        ),
    }
    // A feature or action study can only use what the campaign's intent authorized.
    match study.stage {
        Stage::Feature(factor) => anyhow::ensure!(
            parent.factors.contains(&factor),
            "the campaign did not authorize this factor"
        ),
        Stage::Action(action) => anyhow::ensure!(
            parent.actions.contains(&action),
            "the campaign did not authorize this action"
        ),
        Stage::Screen | Stage::Pilot | Stage::Confirm | Stage::Holdout => {}
    }
    // A confirmation runs under the rule its pilot froze; no other study carries one.
    match (study.stage, rule) {
        (Stage::Confirm | Stage::Holdout, Some(rule)) => anyhow::ensure!(
            rule.is_consistent(),
            "the confirmation rule is not one a pilot could have frozen"
        ),
        (Stage::Confirm | Stage::Holdout, None) => {
            anyhow::bail!("a confirmation needs its pilot's frozen rule")
        }
        (_, Some(_)) => anyhow::bail!("only a confirmation carries a frozen rule"),
        (_, None) => {}
    }
    // A treatment study fills its blocks across the two treatments of its one build.
    let count = if study.arms() == StudyArms::Single {
        2
    } else {
        u32::try_from(tools.len())?
    };
    anyhow::ensure!(
        study.min_attempts > 0 && study.min_attempts.is_multiple_of(count),
        "the study's attempts do not fill whole blocks across its arms"
    );
    anyhow::ensure!(
        study.min_attempts <= parent.max_attempts,
        "the study needs more attempts than the campaign allows"
    );
    let mut child = parent.clone();
    child.tools = tools;
    child.max_attempts = study.min_attempts;
    child.child_study_sha256 = Some(study.study_sha256.clone());
    child.confirmation_rule = rule.cloned();
    child.treatment = study.treatment_spec();
    child.validate()?;
    Ok(child)
}

#[cfg(test)]
#[path = "child_intent_tests.rs"]
mod tests;
