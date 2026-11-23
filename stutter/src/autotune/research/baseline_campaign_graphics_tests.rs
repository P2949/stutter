use super::*;
use crate::autotune::research::{
    child_study::Profile, driver_attempts::tests::baseline_campaign::REPORT_RESOLUTION,
};

/// Through the whole campaign, every verified child carries what its results show of the profile's rendered
/// resolution: the game reports the resolution each applied profile asked for, so every CPU and GPU attempt is
/// confirmed, the long child has none specified, and the bundle's summary says the resolutions were checked.
#[test]
fn every_verified_child_surfaces_its_effective_resolution_as_confirmed_when_the_results_report_it()
{
    let mut run_of = campaign_with_an_action();
    run(&mut run_of);
    let verified = verify_all(&run_of);
    let mut confirmed = 0u64;
    for child in &verified.children {
        let graphics = child
            .effective_graphics
            .as_ref()
            .unwrap_or_else(|| panic!("{} has no effective graphics", child.study));
        assert_eq!(graphics["unproven"], 0, "{}", child.study);
        assert_eq!(graphics["contradicted"], 0, "{}", child.study);
        confirmed += graphics["confirmed"].as_u64().unwrap();
    }
    assert!(confirmed > 0);
    let summary = crate::autotune::research::effective_graphics::summarize(
        &serde_json::json!({ "child_reports": serde_json::to_value(&verified).unwrap() }),
    );
    assert_eq!(summary["confirmed"], confirmed);
    assert_eq!(summary["all_resolutions_confirmed"], true);
}

/// A game whose results do not report the rendered resolution cannot qualify a CPU or GPU profile: the resolution is
/// unproven, never assumed to match, and the study stays unqualified.
#[test]
fn a_profile_study_whose_results_do_not_report_the_resolution_is_not_qualified() {
    REPORT_RESOLUTION.with(|report| report.set(false));
    let mut run_of = campaign_with_an_action();
    run(&mut run_of);
    let dispositions = run_of.driver.dispositions();
    let screen = dispositions
        .iter()
        .find(|d| matches!(d.stage, Stage::Screen) && d.profile == Profile::Cpu)
        .expect("a CPU screening study is planned");
    assert_eq!(
        (screen.result, screen.reason.as_str()),
        (Some(StudyResult::Unqualified), "UnprovenContext"),
        "{dispositions:?}"
    );
}
