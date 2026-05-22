#[test]
fn controller_does_not_compare_raw_score_totals() {
    let source = include_str!("../autotune/controller.rs");
    assert!(
        !source.contains("score_regression_percent"),
        "controller.rs should not use raw score_regression_percent"
    );
    assert!(
        !source.contains("score_improvement_percent"),
        "controller.rs should not use raw score_improvement_percent"
    );
    // Assert the exact field `baseline_score_total` isn't there anymore, verifying the rename to baseline_raw_score_total
    assert!(
        !source.contains("baseline_score_total"),
        "controller.rs should not use baseline_score_total"
    );
}
