use super::*;

#[test]
fn request_pointer_key_requires_matching_issue_and_complete_offsets() {
    let issue_offsets =
        parse_tracepoint_offsets("field:struct request *rq; offset:40; size:8; signed:0;");
    let complete_offsets =
        parse_tracepoint_offsets("field:struct request *rq; offset:40; size:8; signed:0;");

    assert_eq!(
        matching_request_key_offset(&issue_offsets, &complete_offsets),
        Some(40),
    );
}

#[test]
fn request_pointer_key_rejects_mismatched_or_missing_complete_offset() {
    let issue_offsets =
        parse_tracepoint_offsets("field:struct request *rq; offset:40; size:8; signed:0;");
    let mismatched_complete_offsets =
        parse_tracepoint_offsets("field:struct request *rq; offset:48; size:8; signed:0;");
    let missing_complete_offsets =
        parse_tracepoint_offsets("field:dev_t dev; offset:8; size:4; signed:0;");

    assert_eq!(
        matching_request_key_offset(&issue_offsets, &mismatched_complete_offsets),
        None,
    );
    assert_eq!(
        matching_request_key_offset(&issue_offsets, &missing_complete_offsets),
        None,
    );
}

#[test]
fn request_pointer_key_rejects_wrong_size() {
    let issue_offsets = parse_tracepoint_offsets("field:u32 rq; offset:40; size:4; signed:0;");
    let complete_offsets = parse_tracepoint_offsets("field:u32 rq; offset:40; size:4; signed:0;");

    assert_eq!(
        matching_request_key_offset(&issue_offsets, &complete_offsets),
        None,
    );
}
