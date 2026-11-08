use super::*;
use crate::community_rules::audit::class_conflicts;

fn file(rules: &[(&str, &str, &str)]) -> CommunityRulesFile {
    let rules: Vec<String> = rules
        .iter()
        .map(|(name, class, source)| {
            format!(
                r#"{{"name":"{name}","normalized_name":"{name}","type":"Game","stutter_class":"{class}","confidence":0.8,"source_path":"{source}"}}"#
            )
        })
        .collect();
    serde_json::from_str(&format!(
        r#"{{"schema_version":2,"source":{{"name":"t","generated_at":"x"}},"rules":[{}]}}"#,
        rules.join(",")
    ))
    .unwrap()
}

#[test]
fn a_name_mapped_to_two_classes_is_reported_with_both_classes_and_sources() {
    let conflicts = class_conflicts(&file(&[
        ("foo", "Game", "a.rules"),
        ("foo", "Compositor", "b.rules"),
        ("foo", "Game", "c.rules"),
        ("bar", "Game", "a.rules"),
        ("bar", "Game", "b.rules"),
    ]));
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].name, "foo");
    assert_eq!(conflicts[0].classes, ["Compositor", "Game"]);
    assert_eq!(conflicts[0].sources, ["a.rules", "b.rules", "c.rules"]);
}

#[test]
fn a_consistent_or_empty_set_has_no_conflict_and_the_audit_changes_nothing() {
    assert!(class_conflicts(&file(&[])).is_empty());
    let consistent = file(&[("foo", "Game", "a"), ("bar", "Compositor", "a")]);
    assert!(class_conflicts(&consistent).is_empty());
    assert_eq!(consistent.rules.len(), 2);
}
