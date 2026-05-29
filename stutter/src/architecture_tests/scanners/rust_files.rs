use super::*;

#[test]
fn rust_path_extractor_finds_imports_qualified_paths_and_line_numbers() {
    let source = r#"
use crate::{cli, commands::{self, AppCommand}};
use clap::Parser;
fn demo() {
    let _ = crate::daemon::DaemonPolicy::default();
    let _ = super::helper::Thing::new();
    let _ignored = "crate::report::HtmlReportModel";
    // crate::actions::runner::run_audited_action
}
"#;

    let occurrences = rust_path_occurrences(source);

    for (path, line_number) in [
        ("crate::cli", 2),
        ("crate::commands", 2),
        ("crate::commands::AppCommand", 2),
        ("clap::Parser", 3),
        ("crate::daemon::DaemonPolicy::default", 5),
        ("super::helper::Thing::new", 6),
    ] {
        assert!(
            occurrences
                .iter()
                .any(|occurrence| occurrence.path == path && occurrence.line_number == line_number),
            "missing parsed path {path} at line {line_number}; got {occurrences:?}"
        );
    }

    assert!(
        !occurrences
            .iter()
            .any(|occurrence| occurrence.path == "crate::report::HtmlReportModel"),
        "paths inside strings must not be reported"
    );
    assert!(
        !occurrences
            .iter()
            .any(|occurrence| occurrence.path == "crate::actions::runner::run_audited_action"),
        "paths inside comments must not be reported"
    );
}
