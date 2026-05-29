use super::*;

#[test]
fn architecture_violation_message_includes_boundary_path_file_and_line() {
    let file = Path::new("src/actions/mod.rs");
    let occurrence = RustPathOccurrence {
        path: "crate::commands::AppCommand".to_owned(),
        line_number: 17,
    };
    let forbidden = ForbiddenRustPath {
        path: "crate::commands",
        boundary: "actions must not depend on command parsing",
    };

    let message = format_architecture_violation(file, &occurrence, &forbidden);

    assert!(message.contains("src/actions/mod.rs:17"));
    assert!(message.contains("actions must not depend on command parsing"));
    assert!(message.contains("crate::commands"));
    assert!(message.contains("crate::commands::AppCommand"));
}
