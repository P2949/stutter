use std::path::PathBuf;

use crate::test_fixture_builder;

#[test]
#[ignore]
fn regenerate_public_examples_v23() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .expect("stutter crate manifest should have a workspace parent");
    let root = workspace_root
        .join("docs")
        .join("examples")
        .join("artifacts")
        .join("v23");

    test_fixture_builder::write_public_examples_v23(&root)
        .unwrap_or_else(|err| panic!("failed to regenerate public v23 examples: {err:#}"));
}

#[test]
#[ignore]
fn regenerate_validation_corpus() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("runs");
    test_fixture_builder::write_validation_corpus(&root)
        .unwrap_or_else(|err| panic!("failed to regenerate validation corpus: {err:#}"));
}
