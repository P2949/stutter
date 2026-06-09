//! Architecture checks for retired autotune compatibility facades.

#[test]
fn autotune_candidate_compatibility_facade_is_removed() {
    let autotune_mod = include_str!("../autotune/mod.rs");

    assert!(
        !autotune_mod.contains("mod candidate;"),
        "stutter/src/autotune/mod.rs must not reintroduce the internal candidate facade"
    );

    let candidate_facade = crate::architecture_tests::workspace_root()
        .join("stutter")
        .join("src")
        .join("autotune")
        .join("candidate")
        .join("mod.rs");

    assert!(
        !candidate_facade.exists(),
        "{} compatibility facade must stay removed",
        candidate_facade.display()
    );
}
