//! Architecture checks for documented autotune objective semantics.

fn objective_kind_variants() -> Vec<&'static str> {
    let source = include_str!("../autotune/objective.rs");
    let enum_body = source
        .split("pub enum ObjectiveKind")
        .nth(1)
        .and_then(|tail| tail.split_once('}').map(|(body, _)| body))
        .expect("ObjectiveKind enum should be present");

    enum_body
        .lines()
        .filter_map(|line| {
            let variant = line.trim().trim_end_matches(',');
            (!variant.is_empty()
                && !variant.starts_with('#')
                && variant.chars().all(|ch| ch.is_ascii_alphanumeric()))
            .then_some(variant)
        })
        .collect()
}

#[test]
fn autotune_objective_docs_cover_all_objective_kinds() {
    let docs = include_str!("../../../docs/AUTOTUNE_OBJECTIVES.md");
    let variants = objective_kind_variants();

    assert!(
        !variants.is_empty(),
        "ObjectiveKind variant scanner should find documented objective names"
    );

    for variant in variants {
        assert!(
            docs.contains(variant),
            "docs/AUTOTUNE_OBJECTIVES.md must document ObjectiveKind::{variant}"
        );
    }
}
