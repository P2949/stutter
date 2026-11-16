use super::*;

#[test]
fn a_feature_study_enables_exactly_its_factor_and_the_next_study_starts_from_baseline() {
    use crate::autotune::research::{ResearchAction, ResearchFactor, child_study::Stage};
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut parent = parent();
    parent.factors = vec![ResearchFactor::NtSync];
    parent.actions = vec![ResearchAction::CpuAffinityProfile];
    let plan = plan(&parent).unwrap();
    let feature = plan
        .iter()
        .find(|s| matches!(s.stage, Stage::Feature(_)))
        .unwrap()
        .clone();
    let confirm = plan
        .iter()
        .find(|s| matches!(s.stage, Stage::Confirm))
        .unwrap()
        .clone();
    let host = Host::finishing();
    let seen = host.seen_factors.clone();
    let mut now = parent.issued_unix_seconds + 1;
    let mut runner = DriverStudyAttempts::new(
        File::open(root.path()).unwrap(),
        parent.clone(),
        Box::new(host),
        Box::new(move || {
            now += 1;
            now
        }),
        Box::new(|| Ok(())),
        Box::new(|_| std::path::PathBuf::from("/nonexistent")),
        Box::new(|study| Ok(inputs_for(study))),
    );
    run_to_end(&mut runner, &feature);
    run_to_end(&mut runner, &confirm);
    assert_eq!(
        *seen.lock().unwrap(),
        vec![vec![ResearchFactor::NtSync], Vec::<ResearchFactor>::new()],
        "the feature study's factor is enabled for it alone"
    );
    // A feature study the campaign never authorized is refused before any directory exists.
    let mut unauthorized = parent.clone();
    unauthorized.factors.clear();
    let root2 = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root2.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut now2 = parent.issued_unix_seconds + 1;
    let mut refusing = DriverStudyAttempts::new(
        File::open(root2.path()).unwrap(),
        unauthorized,
        Box::new(Host::finishing()),
        Box::new(move || {
            now2 += 1;
            now2
        }),
        Box::new(|| Ok(())),
        Box::new(|_| std::path::PathBuf::from("/nonexistent")),
        Box::new(|study| Ok(inputs_for(study))),
    );
    assert!(refusing.poll(&feature, 100).is_err());
    assert!(!root2.path().join("studies").exists());
}
