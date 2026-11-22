//! Integrity downgrades over the production-composed synthetic campaign: evidence that is removed or
//! corrupted must fail verification, never fall back to weaker rules or read as an optional probe not run.

use std::os::unix::fs::PermissionsExt;

use super::baseline_campaign::{run_campaign, verify};

fn studies_dir(run: &super::baseline_campaign::Finished) -> std::path::PathBuf {
    std::fs::read_link(format!(
        "/proc/self/fd/{}",
        std::os::fd::AsRawFd::as_raw_fd(&run.campaign)
    ))
    .unwrap()
    .join("studies")
}

#[test]
fn a_study_that_lost_its_raw_archive_fails_instead_of_replaying_from_the_game_directory() {
    let run = run_campaign();
    assert!(verify(&run).all_verified());
    // The game's own results directory is still there, which the old fallback would have trusted.
    let first = std::fs::read_dir(studies_dir(&run))
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.join("raw").is_dir())
        .expect("a study with an archive");
    std::fs::remove_dir_all(first.join("raw")).unwrap();
    let verified = verify(&run);
    assert!(!verified.all_verified());
    assert!(
        verified.children.iter().any(|child| child
            .problem
            .as_deref()
            .is_some_and(|p| p.contains("RawArchiveMissing"))),
        "{:?}",
        verified
            .children
            .iter()
            .map(|c| &c.problem)
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_corrupt_telemetry_file_counts_against_the_study_and_is_not_read_as_an_unrun_probe() {
    let run = run_campaign();
    // No direct-frame probe ran in the synthetic campaign: that is the optional probe not collected.
    assert_eq!(verify(&run).telemetry_summary(), None);
    let first = std::fs::read_dir(studies_dir(&run))
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.join("evidence").is_dir())
        .unwrap();
    let file = first.join("telemetry-receipts.json");
    std::fs::write(&file, b"{ not json").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    let (eligible, collected) = verify(&run)
        .telemetry_summary()
        .expect("corrupt evidence is collected");
    assert_eq!(eligible, 0);
    assert!(collected > 0);
}

#[test]
fn admission_counts_the_archive_and_receipts_the_way_the_final_replay_does() {
    let run = run_campaign();
    let studies = std::fs::File::open(studies_dir(&run)).unwrap();
    let uid = crate::syscall::geteuid();
    let sealed: u64 = std::fs::read_dir(studies_dir(&run))
        .unwrap()
        .flatten()
        .map(|entry| {
            serde_json::from_slice::<crate::autotune::research::campaign_budget::Spent>(
                &std::fs::read(entry.path().join("budget.json")).unwrap(),
            )
            .unwrap()
            .evidence_bytes
        })
        .sum();
    let admitted =
        crate::autotune::research::campaign_budget::spent(&studies, uid, 1_000_000).unwrap();
    assert!(
        admitted.evidence_bytes > sealed,
        "{} vs {sealed}",
        admitted.evidence_bytes
    );
}
