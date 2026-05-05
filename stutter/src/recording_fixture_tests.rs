use std::{env, fs};

use crate::report;

#[test]
fn report_rejects_missing_required_artifacts() {
    let mut temp = env::temp_dir();
    temp.push(format!(
        "stutter-test-missing-artifacts-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir_all(&temp).unwrap();

    // Call the report loading helper. It should fail because the directory is empty.
    let result = report::print_report(&temp, false, false, 10, 500, None);

    assert!(result.is_err());
    let err_msg = format!("{:?}", result.err().unwrap()).to_lowercase();

    let contains_marker = err_msg.contains("metadata")
        || err_msg.contains("session")
        || err_msg.contains("recording")
        || err_msg.contains("missing");

    assert!(
        contains_marker,
        "Error message '{}' did not contain any of the required markers (metadata, session, recording, missing)",
        err_msg
    );

    // Cleanup
    let _ = fs::remove_dir_all(temp);
}
