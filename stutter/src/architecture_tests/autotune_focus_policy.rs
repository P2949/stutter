//! Architecture checks for autotune focus-policy candidate gating.

#[test]
fn cpu_affinity_profiles_are_gated_by_candidate_kind_not_profile_name() {
    let source = include_str!("../autotune/controller.rs");

    assert!(
        source.contains("fn candidate_requires_game_focus"),
        "focus policy should centralize game-focus-only candidate classification"
    );
    assert!(
        source.contains("matches!(candidate, CandidateAction::CpuAffinityProfile { .. })"),
        "CPU-affinity profile gating should use the candidate kind, not a profile-name heuristic"
    );
    assert!(
        !source.contains("candidate_looks_like_game_cpu_isolation_profile"),
        "the old name-substring heuristic must not come back"
    );
    for fragile_token in [
        "contains(\"game\")",
        "contains(\"gaming\")",
        "contains(\"isolation\")",
    ] {
        assert!(
            !source.contains(fragile_token),
            "focus policy must not depend on fragile profile-name substring check {fragile_token}"
        );
    }
}
