use std::fs;
use std::path::PathBuf;
use stutter_report::load::{ReportLoadRequest, load_report_model};
use stutter_report::render::text::render_report;

#[test]
fn golden_minimal_report() {
    let input_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minimal/input.json");
    let expected_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minimal/expected.txt");

    let model = load_report_model(&ReportLoadRequest::from_path(&input_path))
        .expect("failed to load input.json fixture");
    
    let actual_text = render_report(&model);

    if std::env::var("UPDATE_GOLDEN").is_ok() {
        fs::write(&expected_path, &actual_text).unwrap();
    }

    let expected_text = fs::read_to_string(&expected_path)
        .expect("failed to read expected.txt fixture (run with UPDATE_GOLDEN=1 to create)");

    assert_eq!(
        actual_text, expected_text,
        "Report rendering output did not match expected output in golden text test. \
         If the change is intentional, run tests with UPDATE_GOLDEN=1 to update the fixture."
    );
}
