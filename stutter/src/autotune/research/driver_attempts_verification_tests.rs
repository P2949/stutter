use super::*;
use crate::autotune::research::child_verify::verify_children;

/// Run the first study to the end and return the campaign directory with its parent intent.
fn finished() -> (tempfile::TempDir, ResearchIntentV1, ChildStudy) {
    let (root, mut runner, study, parent) = attempts(Host::finishing());
    run_to_end(&mut runner, &study);
    (root, parent, study)
}

fn campaign(root: &tempfile::TempDir) -> File {
    File::open(root.path()).unwrap()
}

fn lookup(_: ProtonSource) -> std::path::PathBuf {
    "/nonexistent".into()
}

#[test]
fn a_finished_child_reconstructs_and_its_missing_raw_results_are_reported_not_assumed() {
    let (root, parent, study) = finished();
    let uid = crate::syscall::geteuid();
    let result = verify_children(&campaign(&root), uid, &parent, &lookup).unwrap();
    assert_eq!(result.children.len(), 1);
    let child = &result.children[0];
    assert_eq!(child.study, study.study_sha256[..16]);
    assert!(child.reconstruction, "{:?}", child.problem);
    // The fake host sealed no native results, so nothing can replay: not verified.
    assert!(!child.raw_files_verified);
    assert!(!result.all_verified());
}

#[test]
fn a_child_records_the_cache_cohort_its_arms_ran_in() {
    let (root, parent, _) = finished();
    let result = verify_children(
        &campaign(&root),
        crate::syscall::geteuid(),
        &parent,
        &lookup,
    )
    .unwrap();
    assert_eq!(
        result.children[0].prefix_cohort.as_ref().unwrap()["cohort"],
        "warm"
    );
}

#[test]
fn a_campaign_without_children_has_nothing_verified() {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let result = verify_children(
        &campaign(&root),
        crate::syscall::geteuid(),
        &parent(),
        &lookup,
    )
    .unwrap();
    assert!(result.children.is_empty());
    assert!(!result.all_verified());
}

#[test]
fn a_tampered_report_a_forged_intent_or_an_unexpected_entry_fails_reconstruction() {
    let uid = crate::syscall::geteuid();
    // Tamper with the sealed report.
    let (root, parent, study) = finished();
    let dir = root.path().join("studies").join(&study.study_sha256[..16]);
    let report = std::fs::read_to_string(dir.join("report.json")).unwrap();
    std::fs::write(
        dir.join("report.json"),
        report.replace("\"attempts_completed\":", "\"attempts_completed\":9"),
    )
    .unwrap();
    let tampered = verify_children(&campaign(&root), uid, &parent, &lookup).unwrap();
    assert!(!tampered.children[0].reconstruction);
    // A child whose intent names a tool the campaign never authorized.
    let (root, parent, study) = finished();
    let dir = root.path().join("studies").join(&study.study_sha256[..16]);
    let mut forged: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("intent.json")).unwrap()).unwrap();
    forged["tools"][0]["identity_sha256"] = "9".repeat(64).into();
    std::fs::write(dir.join("intent.json"), forged.to_string()).unwrap();
    let foreign = verify_children(&campaign(&root), uid, &parent, &lookup).unwrap();
    assert!(!foreign.children[0].reconstruction);
    assert!(
        foreign.children[0]
            .problem
            .as_deref()
            .unwrap()
            .contains("campaign's own intent")
    );
    // A child bound to a different study digest than its directory name.
    let (root, parent, study) = finished();
    let dir = root.path().join("studies").join(&study.study_sha256[..16]);
    let mut rebound: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("intent.json")).unwrap()).unwrap();
    rebound["child_study_sha256"] = "7".repeat(64).into();
    std::fs::write(dir.join("intent.json"), rebound.to_string()).unwrap();
    assert!(
        !verify_children(&campaign(&root), uid, &parent, &lookup)
            .unwrap()
            .children[0]
            .reconstruction
    );
    // An entry that is not a study digest.
    let (root, parent, _) = finished();
    std::fs::create_dir(root.path().join("studies/stray")).unwrap();
    let stray = verify_children(&campaign(&root), uid, &parent, &lookup).unwrap();
    assert!(
        stray
            .children
            .iter()
            .any(|c| c.study == "stray" && !c.reconstruction)
    );
}
