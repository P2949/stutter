use stutter::api::capabilities::capability_registry;

use super::{MANIFEST_TEXT, manifest};
use crate::validation_campaign::manifest::CampaignManifest;

fn edited(from: &str, to: &str) -> String {
    assert!(MANIFEST_TEXT.contains(from), "{from}");
    MANIFEST_TEXT.replacen(from, to, 1)
}

#[test]
fn the_committed_manifest_parses_and_validates() {
    let manifest = manifest();
    assert_eq!(manifest.release.tag, "v0.1.0");
    assert_eq!(manifest.kernels.required_distinct, 3);
    assert_eq!(manifest.score.levels_ms, [0, 1, 2, 4, 8, 16]);
}

#[test]
fn the_capability_policy_partitions_exactly_the_registry() {
    let policy = manifest().policy.capabilities;
    let mut listed = policy
        .supported
        .iter()
        .chain(&policy.experimental)
        .chain(&policy.internal)
        .cloned()
        .collect::<Vec<_>>();
    listed.sort();
    let mut registry = capability_registry()
        .iter()
        .map(|c| c.id.to_owned())
        .collect::<Vec<_>>();
    registry.sort();
    assert_eq!(
        listed, registry,
        "the policy must name every capability exactly once"
    );
}

#[test]
fn invalid_manifests_are_rejected() {
    for (from, to) in [
        ("tag = \"v0.1.0\"", "tag = \"v0.2.0\""),
        ("version = \"0.1.0\"", "version = \"0.1\""),
        (
            "levels_ms = [0, 1, 2, 4, 8, 16]",
            "levels_ms = [1, 2, 4, 8]",
        ),
        (
            "levels_ms = [0, 1, 2, 4, 8, 16]",
            "levels_ms = [0, 4, 2, 8]",
        ),
        ("required_distinct = 3", "required_distinct = 0"),
        ("min_positive = 10", "min_positive = 0"),
        ("schema_version = 1\nname", "schema_version = 9\nname"),
        (
            "    \"diagnose\",\n",
            "    \"diagnose\",\n    \"doctor\",\n",
        ),
    ] {
        assert!(
            CampaignManifest::parse(&edited(from, to)).is_err(),
            "{from} -> {to}"
        );
    }
    assert!(CampaignManifest::parse("not toml = =").is_err());
}

#[test]
fn the_digest_changes_with_any_byte() {
    use crate::evidence_bundle::sha256::sha256_hex;
    assert_ne!(
        sha256_hex(MANIFEST_TEXT.as_bytes()),
        sha256_hex(edited("rounds = 12", "rounds = 13").as_bytes())
    );
}
