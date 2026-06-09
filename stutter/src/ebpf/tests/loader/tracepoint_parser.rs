use super::*;

#[test]
fn parses_tracepoint_field_offsets() {
    let format = r#"
field:unsigned short common_type; offset:0; size:2; signed:0;
field:char prev_comm[16]; offset:8; size:16; signed:1;
field:pid_t prev_pid; offset:24; size:4; signed:1;
field:int prev_prio; offset:28; size:4; signed:1;
field:long prev_state; offset:32; size:8; signed:1;
field:char next_comm[16]; offset:40; size:16; signed:1;
field:pid_t next_pid; offset:56; size:4; signed:1;
field:int next_prio; offset:60; size:4; signed:1;
"#;

    let offsets = parse_tracepoint_offsets(format);

    assert_eq!(offsets.get("next_comm").map(|f| f.offset), Some(40));
    assert_eq!(offsets.get("next_pid").map(|f| f.offset), Some(56));
    assert_eq!(offsets.get("next_prio").map(|f| f.offset), Some(60));
    assert_eq!(offsets.get("next_comm").map(|f| f.size), Some(16));
    assert_eq!(offsets.get("next_pid").map(|f| f.size), Some(4));
    assert_eq!(offsets.get("next_prio").map(|f| f.size), Some(4));
}

#[test]
fn parse_tracepoint_fields_preserves_original_declaration() {
    let format = "    field:char next_comm[16]; offset:40; size:16; signed:1;\n";

    let fields = parse_tracepoint_offsets(format);
    let field = fields.get("next_comm").unwrap();

    assert_eq!(field.name, "next_comm");
    assert_eq!(field.offset, 40);
    assert_eq!(field.size, 16);
    assert!(field.signed);
    assert_eq!(
        field.declaration,
        "field:char next_comm[16]; offset:40; size:16; signed:1;",
    );
}
