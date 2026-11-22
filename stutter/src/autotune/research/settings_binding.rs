//! Which settings digest each completed attempt must have run with. The profile hook records one digest
//! per arm, in the campaign's tool order; an attempt is held to the digest of its own arm, not to any
//! arm's, and a study that applied a profile must have a complete record of it.

use std::collections::BTreeMap;

use super::{
    ResearchIntentV1, child_study::ChildStudy, native_context::ProfileConformance,
    report::AttemptEvidenceV1,
};

/// What the study's record says about the settings the attempt's arm should have left.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ArmSettings<'a> {
    /// No profile was applied (none exists for the stage, or an older study that never recorded one).
    NotApplied,
    /// The arm's own digest.
    Digest(&'a str),
    /// A profile was applied but the record cannot be tied to this arm: absent, partial or reordered.
    Unbound,
}

/// The digest an attempt on `arm` must match. `recorded` is `None` when the study has no readable record;
/// `modern` is true for a study that keeps a campaign raw archive, which always records what it applied.
pub(crate) fn expected_for<'a>(
    recorded: Option<&'a [String]>,
    intent: &ResearchIntentV1,
    study: &ChildStudy,
    arm: super::ProtonSource,
    modern: bool,
) -> ArmSettings<'a> {
    let profile_applied = !matches!(
        super::profile_hook::production_spec(study.profile),
        Ok(None)
    ) && study.stage != super::child_study::Stage::Init;
    let Some(recorded) = recorded.filter(|digests| !digests.is_empty()) else {
        return if modern && profile_applied {
            ArmSettings::Unbound
        } else {
            ArmSettings::NotApplied
        };
    };
    if recorded.len() != intent.tools.len() {
        return ArmSettings::Unbound;
    }
    intent
        .tools
        .iter()
        .position(|tool| tool.source == arm)
        .and_then(|index| recorded.get(index))
        .map_or(ArmSettings::Unbound, |digest| ArmSettings::Digest(digest))
}

/// Every completed attempt's conformance to its study's frozen profile, held to its own arm's settings.
/// The live outcome and every verifier use this one function.
pub(crate) fn conformances(
    evidence: &BTreeMap<u32, AttemptEvidenceV1>,
    recorded: Option<&[String]>,
    intent: &ResearchIntentV1,
    study: &ChildStudy,
    modern: bool,
) -> Vec<ProfileConformance> {
    let arms: BTreeMap<u32, super::ProtonSource> = super::driver::build_schedule(intent)
        .map(|slots| {
            slots
                .into_iter()
                .map(|slot| (slot.index, slot.arm))
                .collect()
        })
        .unwrap_or_default();
    evidence
        .values()
        .map(|item| {
            let expected = match arms.get(&item.slot_index) {
                Some(arm) => expected_for(recorded, intent, study, *arm, modern),
                None => ArmSettings::Unbound,
            };
            super::init_receipt::conformance_of(item, &expected, study)
        })
        .collect()
}
