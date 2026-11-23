use serde_json::json;

use super::*;

fn child(
    confirmed: u64,
    contradicted: u64,
    unproven: u64,
    not_specified: u64,
) -> serde_json::Value {
    json!({ "effective_graphics": {
        "profile": "cpu", "confirmed": confirmed, "contradicted": contradicted,
        "unproven": unproven, "not_specified": not_specified,
    }})
}

fn response(children: Vec<serde_json::Value>) -> serde_json::Value {
    json!({ "child_reports": { "children": children } })
}

#[test]
fn the_bundle_says_whether_every_resolution_was_confirmed_and_never_implies_a_check_that_was_not_made()
 {
    let all = summarize(&response(vec![
        child(8, 0, 0, 0),
        child(8, 0, 0, 0),
        child(0, 0, 0, 8),
    ]));
    assert_eq!(all["confirmed"], 16);
    assert_eq!(all["all_resolutions_confirmed"], true);
    // A single attempt whose result did not report the resolution leaves the claim unmade.
    let silent = summarize(&response(vec![child(8, 0, 0, 0), child(7, 0, 1, 0)]));
    assert_eq!(
        (
            silent["unproven"].clone(),
            silent["all_resolutions_confirmed"].clone()
        ),
        (json!(1), json!(false))
    );
    // A contradiction is not confirmed either.
    assert_eq!(
        summarize(&response(vec![child(7, 1, 0, 0)]))["all_resolutions_confirmed"],
        false
    );
    // Nothing verified at all confirms nothing.
    assert_eq!(
        summarize(&response(Vec::new()))["all_resolutions_confirmed"],
        false
    );
    assert_eq!(summarize(&json!({}))["confirmed"], 0);
}
