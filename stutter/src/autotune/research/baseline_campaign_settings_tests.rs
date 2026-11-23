use super::*;

#[test]
fn a_profile_that_left_other_settings_than_the_game_ran_with_is_unqualified() {
    use crate::autotune::research::child_study::Stage;
    let run = run_campaign_with_profile(Some("e".repeat(64)));
    let all = run.driver.dispositions();
    let screen = all.iter().find(|d| d.stage == Stage::Screen).unwrap();
    assert_eq!(screen.result, Some(StudyResult::Unqualified), "{all:?}");
    assert_eq!(screen.reason, "ProfileViolated");
}

#[test]
fn a_study_that_applied_a_profile_cannot_lose_or_scramble_its_settings_record() {
    let run = run_campaign_with_profile(Some("c".repeat(64)));
    let campaign_path = std::fs::read_link(format!(
        "/proc/self/fd/{}",
        std::os::fd::AsRawFd::as_raw_fd(&run.campaign)
    ))
    .unwrap();
    let receipt = std::fs::read_dir(campaign_path.join("studies"))
        .unwrap()
        .flatten()
        .map(|entry| {
            entry
                .path()
                .join(crate::autotune::research::init_receipt::SETTINGS_RECEIPT)
        })
        .find(|path| path.is_file())
        .expect("a study recorded the settings its profile left");
    let original = std::fs::read(&receipt).unwrap();
    std::fs::set_permissions(&receipt, std::fs::Permissions::from_mode(0o600)).unwrap();
    let digests: Vec<String> =
        crate::autotune::research::init_receipt::read_settings_receipt(&original).unwrap();
    // One arm's digest is wrong while another's matches: membership in the list is not enough.
    let mut scrambled = digests.clone();
    scrambled[0] = "e".repeat(64);
    // A partial record (an arm missing) cannot be tied to every arm.
    let partial = digests[..digests.len() - 1].to_vec();
    for bad in [scrambled, partial] {
        std::fs::write(
            &receipt,
            crate::autotune::research::init_receipt::settings_receipt_bytes(&bad).unwrap(),
        )
        .unwrap();
        assert!(
            verify(&run).children.iter().any(|c| c.problem.is_some()),
            "{bad:?}"
        );
    }
    // A record that is gone from a study that applied a profile is not a study with no profile.
    std::fs::remove_file(&receipt).unwrap();
    assert!(verify(&run).children.iter().any(|c| c.problem.is_some()));
}
