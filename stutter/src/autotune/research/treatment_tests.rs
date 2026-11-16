use super::*;
use crate::autotune::research::{ProtonSource, driver::build_schedule, tests::intent};

fn treated(spec: TreatmentSpec) -> ResearchIntentV1 {
    let mut intent = intent();
    intent.factors = vec![ResearchFactor::NtSync];
    intent.actions = vec![ResearchAction::CpuAffinityProfile];
    intent.child_study_sha256 = Some("ab".repeat(32));
    intent.treatment = Some(spec);
    intent.max_attempts = 8;
    intent
}

const FACTOR: TreatmentSpec = TreatmentSpec::Factor(ResearchFactor::NtSync);

#[test]
fn an_intent_without_a_treatment_keeps_its_bytes_and_digest() {
    let plain = intent();
    let json = serde_json::to_string(&plain).unwrap();
    assert!(
        !json.contains("treatment"),
        "an absent treatment is not serialized"
    );
    let back: ResearchIntentV1 = serde_json::from_str(&json).unwrap();
    assert_eq!(back.digest().unwrap(), plain.digest().unwrap());
    assert_eq!(treatments_of(&plain).unwrap(), None);
}

#[test]
fn one_build_is_scheduled_as_balanced_off_on_blocks() {
    let intent = treated(FACTOR);
    intent.validate().unwrap();
    let schedule = build_schedule(&intent).unwrap();
    let treatments = treatments_of(&intent).unwrap().unwrap();
    assert_eq!(schedule.len(), 8);
    assert_eq!(treatments.len(), 8);
    assert!(schedule.iter().all(|slot| slot.arm == ProtonSource::Ge));
    for block in treatments.chunks(2) {
        assert_ne!(block[0], block[1], "every block holds one off and one on");
    }
    // Regenerated identically, and distinct from a different treatment's digest.
    assert_eq!(treatments, treatments_of(&intent).unwrap().unwrap());
    let other = treated(TreatmentSpec::Action(ResearchAction::CpuAffinityProfile));
    assert_ne!(other.digest().unwrap(), intent.digest().unwrap());
}

#[test]
fn a_treatment_needs_one_build_whole_blocks_and_an_authorized_switch() {
    let mut two_builds = treated(FACTOR);
    two_builds
        .tools
        .push(crate::autotune::research::ResearchTool {
            source: ProtonSource::ValveStable,
            identity_sha256: "b".repeat(64),
        });
    assert!(treatments_of(&two_builds).is_err());
    let mut odd = treated(FACTOR);
    odd.max_attempts = 7;
    assert!(treatments_of(&odd).is_err());
    let mut unauthorized = treated(FACTOR);
    unauthorized.factors.clear();
    assert_eq!(
        unauthorized.validate().unwrap_err().to_string(),
        "invalid treatment"
    );
    let mut not_a_child = treated(FACTOR);
    not_a_child.child_study_sha256 = None;
    assert_eq!(
        not_a_child.validate().unwrap_err().to_string(),
        "invalid treatment"
    );
}
