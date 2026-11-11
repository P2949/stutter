use super::*;

#[test]
fn parses_nested_objects_comments_and_escaped_strings() {
    let document = VdfDocument::parse(
        br#""compatibilitytools" { "compat_tools" { "tool-id" { "install_path" "." "display_name" "A \"custom\" tool" } } // note
        }"#,
    )
    .unwrap();
    assert_eq!(document.name, "compatibilitytools");
    let tool = document.root_object().unwrap()["compat_tools"]
        .object()
        .unwrap()["tool-id"]
        .object()
        .unwrap();
    assert_eq!(tool["install_path"].string(), Some("."));
    assert_eq!(tool["display_name"].string(), Some("A \"custom\" tool"));
}

#[test]
fn rejects_duplicate_keys_at_every_depth_and_malformed_values() {
    for input in [
        br#""AppState" { "appid" "1" "APPID" "2" }"#.as_slice(),
        br#""AppState" { "nested" { "id" "1" "Id" "2" } }"#,
        br#""AppState" { "appid" "1" "dangling" }"#,
        br#""AppState" { "appid" "unterminated }"#,
        b"unquoted { value }",
    ] {
        assert!(VdfDocument::parse(input).is_err(), "accepted {input:?}");
    }
    assert!(VdfDocument::parse(br#""AppState" { "appid" { "nested" "1" } }"#).is_ok());
}

#[test]
fn rejects_unsupported_escapes_trailing_content_and_deep_input() {
    assert!(VdfDocument::parse(br#""root" { "value" "bad\nvalue" }"#).is_err());
    assert!(VdfDocument::parse(br#""root" { } "extra" { }"#).is_err());
    let roots =
        VdfDocument::parse_all(br#""manifest" { "version" "1" } "signature" { "root" "digest" }"#)
            .unwrap();
    assert_eq!(
        roots
            .iter()
            .map(|root| root.name.as_str())
            .collect::<Vec<_>>(),
        ["manifest", "signature"]
    );
    assert!(VdfDocument::parse_all(br#""root" { } "ROOT" { }"#).is_err());

    let mut deep = String::from("\"root\" {");
    for _ in 0..=MAX_DEPTH {
        deep.push_str("\"nested\" {");
    }
    for _ in 0..=MAX_DEPTH {
        deep.push('}');
    }
    assert!(VdfDocument::parse(deep.as_bytes()).is_err());
    assert!(VdfDocument::parse(&vec![b' '; MAX_INPUT_BYTES + 1]).is_err());
}
