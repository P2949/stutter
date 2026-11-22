use serde::Deserialize;

use super::*;

#[derive(Debug, Deserialize, PartialEq)]
struct Rec {
    at: u64,
}

fn write(dir: &std::path::Path, name: &str, body: &str) {
    std::fs::write(dir.join(name), body).unwrap();
}

#[test]
fn the_newest_valid_record_wins_and_other_files_are_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let get = || newest::<Rec>(dir.path(), "rec-", |r| r.at);
    assert_eq!(get(), Newest::None);
    write(dir.path(), "rec-1.json", r#"{"at":5}"#);
    write(dir.path(), "rec-2.json", r#"{"at":9}"#);
    write(dir.path(), "other.json", "{ not even json");
    assert_eq!(get(), Newest::Found(Rec { at: 9 }));
    assert_eq!(
        newest::<Rec>(&dir.path().join("absent"), "rec-", |r| r.at),
        Newest::None
    );
}

#[test]
fn a_present_record_that_is_corrupt_oversized_or_a_link_is_invalid_not_absent() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "rec-1.json", r#"{"at":5}"#);
    write(dir.path(), "rec-2.json", "{");
    assert_eq!(newest::<Rec>(dir.path(), "rec-", |r| r.at), Newest::Invalid);
    std::fs::remove_file(dir.path().join("rec-2.json")).unwrap();
    std::os::unix::fs::symlink(dir.path().join("rec-1.json"), dir.path().join("rec-3.json"))
        .unwrap();
    assert_eq!(newest::<Rec>(dir.path(), "rec-", |r| r.at), Newest::Invalid);
    std::fs::remove_file(dir.path().join("rec-3.json")).unwrap();
    write(
        dir.path(),
        "rec-4.json",
        &format!("{{\"at\":1,\"pad\":\"{}\"}}", "x".repeat(300_000)),
    );
    assert_eq!(newest::<Rec>(dir.path(), "rec-", |r| r.at), Newest::Invalid);
}
