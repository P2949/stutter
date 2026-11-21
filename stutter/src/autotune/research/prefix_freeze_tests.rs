use super::*;
use crate::autotune::research::{ProtonSource, ResearchTool, tests::intent};

fn campaign() -> ResearchIntentV1 {
    let mut intent = intent();
    intent.tools.push(ResearchTool {
        source: ProtonSource::ValveStable,
        identity_sha256: "b".repeat(64),
    });
    intent
}

fn warm(root: &Path, dir: &str) {
    let pfx = root.join(dir).join("pfx");
    std::fs::create_dir_all(&pfx).unwrap();
    for name in ["system.reg", "user.reg", "userdef.reg"] {
        std::fs::write(pfx.join(name), b"x").unwrap();
    }
}

#[test]
fn only_the_cold_arms_are_frozen_in_the_intents_own_order_and_the_digest_changes() {
    let root = tempfile::tempdir().unwrap();
    let intent = campaign();
    // Ge has no prefix at all (cold); Valve Stable's is initialized (warm).
    warm(root.path(), "valve_stable");
    let frozen = with_cold_arms(&intent, root.path()).unwrap();
    assert_eq!(frozen.prefix_init_arms, [ProtonSource::Ge]);
    assert_ne!(frozen.digest().unwrap(), intent.digest().unwrap());
    // Everything warm: nothing to initialize, and the intent (and its digest) is exactly the original.
    warm(root.path(), "ge");
    let none = with_cold_arms(&intent, root.path()).unwrap();
    assert!(none.prefix_init_arms.is_empty());
    assert_eq!(none.digest().unwrap(), intent.digest().unwrap());
}

#[test]
fn a_partly_initialized_prefix_is_refused_rather_than_guessed() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("ge/pfx")).unwrap();
    std::fs::write(root.path().join("ge/pfx/system.reg"), b"x").unwrap();
    let error = with_cold_arms(&campaign(), root.path()).unwrap_err();
    assert!(format!("{error}").contains("partly initialized"));
}

#[test]
fn a_frozen_intent_plans_the_initialization_study_before_every_screen() {
    use crate::autotune::research::child_study::{Stage, plan};
    let root = tempfile::tempdir().unwrap();
    let frozen = with_cold_arms(&campaign(), root.path()).unwrap();
    assert_eq!(frozen.prefix_init_arms.len(), 2, "both arms are cold");
    let studies = plan(&frozen).unwrap();
    assert_eq!(studies[0].stage, Stage::Init);
    assert!(
        studies
            .iter()
            .filter(|study| study.stage == Stage::Screen)
            .all(|screen| screen.depends_on == [studies[0].study_sha256.clone()])
    );
}
