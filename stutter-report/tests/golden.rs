use std::{fs, path::PathBuf};

use stutter_report::{
    load::{ReportLoadRequest, load_report_model},
    render::text::render_report,
};

fn run_golden_test(fixture_name: &str) {
    let fixture_dir =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/{fixture_name}"));
    let input_path = fixture_dir.join("input.json");
    let expected_path = fixture_dir.join("expected.txt");

    let model = load_report_model(&ReportLoadRequest::from_path(&input_path))
        .unwrap_or_else(|err| panic!("failed to load {fixture_name}/input.json: {err}"));

    let actual_text = render_report(&model);

    if std::env::var("UPDATE_GOLDEN").is_ok() {
        fs::write(&expected_path, &actual_text).unwrap();
    }

    let expected_text = fs::read_to_string(&expected_path).unwrap_or_else(|_| {
        panic!(
            "failed to read {fixture_name}/expected.txt (run with UPDATE_GOLDEN=1 to create)"
        )
    });

    assert_eq!(
        actual_text, expected_text,
        "Report rendering output for {fixture_name} did not match golden fixture. \
         If the change is intentional, run tests with UPDATE_GOLDEN=1 to update."
    );
}

#[test]
fn golden_minimal_report() {
    run_golden_test("minimal");
}

#[test]
fn golden_with_clusters_report() {
    run_golden_test("with_clusters");
}
