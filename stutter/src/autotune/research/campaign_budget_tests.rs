use std::os::unix::fs::PermissionsExt;

use super::*;

fn private_dir(parent: &std::path::Path, name: &str) -> (std::path::PathBuf, File) {
    let path = parent.join(name);
    std::fs::create_dir(&path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let file = File::open(&path).unwrap();
    (path, file)
}

fn uid() -> u32 {
    crate::syscall::geteuid()
}

#[test]
fn children_are_charged_what_they_recorded_and_an_unrecorded_one_the_whole_active_budget() {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let (_, studies) = private_dir(root.path(), "studies");
    let studies_path = root.path().join("studies");

    // Nothing has run: nothing is spent.
    assert_eq!(spent(&studies, uid(), 1000).unwrap(), Spent::default());

    // Two children recorded their own use; the campaign has spent the sum.
    for (name, active) in [("aaaaaaaaaaaaaaaa", 300), ("bbbbbbbbbbbbbbbb", 450)] {
        let (_, study) = private_dir(&studies_path, name);
        let (_, evidence) = private_dir(&studies_path.join(name), "evidence");
        record(&study, &evidence, uid(), active).unwrap();
    }
    assert_eq!(
        spent(&studies, uid(), 1000).unwrap().active_seconds,
        750,
        "the children's use adds up; it does not reset per child"
    );

    // A child that left no record (a crash, or one still running at a restart) has an unknown cost.
    private_dir(&studies_path, "cccccccccccccccc");
    assert_eq!(spent(&studies, uid(), 1000).unwrap().active_seconds, 1750);
}

#[test]
fn a_recorded_budget_cannot_be_rewritten_to_something_else() {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let (_, study) = private_dir(root.path(), "study");
    let (_, evidence) = private_dir(&root.path().join("study"), "evidence");
    record(&study, &evidence, uid(), 100).unwrap();
    // The same record again is fine (a retried write); a different one is refused.
    record(&study, &evidence, uid(), 100).unwrap();
    assert!(record(&study, &evidence, uid(), 5).is_err());
}
