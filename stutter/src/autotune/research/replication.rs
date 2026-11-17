//! Whether a held-out repetition reproduces its confirmation. A holdout is a fresh, separately
//! seeded repetition of the confirmation under the same frozen rule. Both can be methodologically
//! valid and still disagree, so "the holdout qualified" says only that it was a sound measurement;
//! a reproduced result needs the two to reach the same numeric verdict for each build compared. A
//! confirmation that said "improved" and a holdout that said "regressed" is a contradiction, not two
//! passes, and an inconclusive repetition reproduces nothing.

use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Replication {
    /// Every compared build reached the same directional verdict (improved, or regressed) twice.
    Reproduced,
    /// Every compared build was twice equivalent, or twice without a useful effect.
    ReproducedNull,
    /// Some build's verdict differs between the confirmation and the holdout.
    Contradicted,
    /// One side has no numeric verdict, or reached an inconclusive one: nothing is reproduced.
    Undetermined,
}

impl Replication {
    /// Whether a recorded outcome (by its `name`) is one that supports the confirmation's
    /// conclusion; the verifier reads outcomes back by name.
    pub fn name_supports(name: &str) -> bool {
        name == Self::Reproduced.name() || name == Self::ReproducedNull.name()
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Reproduced => "reproduced",
            Self::ReproducedNull => "reproduced_null",
            Self::Contradicted => "contradicted",
            Self::Undetermined => "undetermined",
        }
    }
}

/// The numeric verdict of each compared build in a child report, by its `source`.
fn verdicts(report: &Value) -> Vec<(String, String)> {
    report["arms"]
        .as_array()
        .map(|arms| {
            arms.iter()
                .filter_map(|arm| {
                    Some((
                        arm["source"].as_str()?.to_owned(),
                        arm["numeric_verdict"].as_str()?.to_owned(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Compare a confirmation's child report with its holdout's.
pub fn compare(confirmation: &Value, holdout: &Value) -> Replication {
    let (first, second) = (verdicts(confirmation), verdicts(holdout));
    if first.is_empty() || second.is_empty() {
        return Replication::Undetermined;
    }
    let mut kinds = Vec::new();
    for (source, verdict) in &first {
        let Some((_, again)) = second.iter().find(|(other, _)| other == source) else {
            // A build compared once and not again cannot be said to have been reproduced.
            return Replication::Undetermined;
        };
        if again != verdict {
            return Replication::Contradicted;
        }
        kinds.push(verdict.as_str());
    }
    if kinds.contains(&"inconclusive") {
        return Replication::Undetermined;
    }
    if kinds
        .iter()
        .all(|kind| matches!(*kind, "improved" | "regressed"))
    {
        Replication::Reproduced
    } else if kinds
        .iter()
        .all(|kind| matches!(*kind, "equivalent" | "no_useful_effect"))
    {
        Replication::ReproducedNull
    } else {
        // Different kinds of conclusion across builds (one improved, one equivalent): each agrees
        // with itself but the study did not reproduce a single conclusion.
        Replication::Undetermined
    }
}

/// Whether the holdout child `name` reproduces its confirmation, from the sealed child reports
/// (`report_of` reads a child's report by its directory name). `None` when `name` is not a holdout in
/// the campaign's plan; an unreadable report on either side is `Undetermined`.
pub fn replication_of(
    parent: &super::ResearchIntentV1,
    name: &str,
    report_of: &dyn Fn(&str) -> Option<Value>,
) -> Option<Replication> {
    use super::child_study::{Stage, plan};
    let studies = plan(parent).ok()?;
    let holdout = studies
        .iter()
        .find(|study| study.stage == Stage::Holdout && study.study_sha256.starts_with(name))?;
    let confirmation = holdout.depends_on.iter().find_map(|digest| {
        studies
            .iter()
            .find(|study| study.stage == Stage::Confirm && &study.study_sha256 == digest)
    });
    let (Some(confirmation), Some(own)) = (confirmation, report_of(name)) else {
        return Some(Replication::Undetermined);
    };
    let first = report_of(confirmation.study_sha256.get(..16).unwrap_or_default());
    Some(first.map_or(Replication::Undetermined, |first| compare(&first, &own)))
}

#[cfg(test)]
#[path = "replication_tests.rs"]
mod tests;
