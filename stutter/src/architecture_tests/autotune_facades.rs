//! Architecture checks for retired autotune compatibility facades.

use std::path::Path;

#[test]
fn autotune_candidate_compatibility_facade_is_removed() {
    let autotune_mod = include_str!("../autotune/mod.rs");

    assert!(
        !autotune_mod.contains("mod candidate;"),
        "stutter/src/autotune/mod.rs must not reintroduce the internal candidate facade"
    );
    assert!(
        !Path::new("src/autotune/candidate/mod.rs").exists(),
        "stutter/src/autotune/candidate/mod.rs compatibility facade must stay removed"
    );
}
