use super::*;

#[test]
fn event_stream_warning_is_absent_without_errors() {
    assert!(event_stream_warning(0, None).is_none());
}

#[test]
fn event_stream_warning_includes_count_and_first_error() {
    let warning = event_stream_warning(2, Some("spike_events: No space left on device")).unwrap();

    assert!(warning.contains("2 write error"));
    assert!(warning.contains("event artifact files may be incomplete"));
    assert!(warning.contains("spike_events: No space left on device"));
}

#[test]
fn event_stream_warning_handles_missing_first_error() {
    let warning = event_stream_warning(1, None).unwrap();

    assert!(warning.contains("1 write error"));
    assert!(warning.contains("first error was not recorded"));
}
