use super::*;

/// Through the whole campaign, every verified child carries what its results show of the profile's rendered
/// resolution: the synthetic native files report none, so each CPU and GPU attempt is typed unproven (never assumed
/// confirmed), the long child has none specified, and the bundle's summary does not claim the resolutions were checked.
#[test]
fn every_verified_child_surfaces_its_effective_resolution_as_unproven_when_the_results_do_not_report_it()
 {
    let mut run_of = campaign_with_an_action();
    run(&mut run_of);
    let verified = verify_all(&run_of);
    let mut unproven = 0u64;
    for child in &verified.children {
        let graphics = child
            .effective_graphics
            .as_ref()
            .unwrap_or_else(|| panic!("{} has no effective graphics", child.study));
        assert_eq!(graphics["confirmed"], 0);
        assert_eq!(graphics["contradicted"], 0);
        unproven += graphics["unproven"].as_u64().unwrap();
    }
    assert!(unproven > 0);
    let summary = crate::autotune::research::effective_graphics::summarize(
        &serde_json::json!({ "child_reports": serde_json::to_value(&verified).unwrap() }),
    );
    assert_eq!(summary["unproven"], unproven);
    assert_eq!(summary["all_resolutions_confirmed"], false);
}
