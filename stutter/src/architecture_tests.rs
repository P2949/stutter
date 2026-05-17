use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DependencyMatrixEntry {
    subsystem: &'static str,
    may_depend_on: &'static [&'static str],
    must_not_depend_on: &'static [&'static str],
}

const KNOWN_TOP_LEVEL_ARCHITECTURE_MODULES: &[&str] = &[
    "actions",
    "agent",
    "autotune",
    "cli",
    "commands",
    "config",
    "daemon",
    "events",
    "focus",
    "process_tree",
    "recorder",
    "report",
    "system",
];

const ARCHITECTURE_DEPENDENCY_MATRIX: &[DependencyMatrixEntry] = &[
    DependencyMatrixEntry {
        subsystem: "cli",
        may_depend_on: &[
            "commands",
            "commands::input",
            "config",
            "daemon",
            "service",
            "validate",
        ],
        must_not_depend_on: &[
            "actions::runner",
            "autotune::live_experiment",
            "daemon::runtime",
            "recorder::live",
        ],
    },
    DependencyMatrixEntry {
        subsystem: "commands",
        may_depend_on: &[
            "actions",
            "agent",
            "artifacts",
            "autotune",
            "config",
            "daemon",
            "doctor",
            "events",
            "presets",
            "probe_activation",
            "probe_registry",
            "process_tree",
            "recorder",
            "release",
            "remote",
            "report",
            "scenario",
            "service",
            "session",
            "session_io",
            "system",
            "validate",
        ],
        must_not_depend_on: &[
            "actions::runner without daemon policy",
            "autotune provider mutation",
            "undocumented persistence paths",
        ],
    },
    DependencyMatrixEntry {
        subsystem: "agent",
        may_depend_on: &[
            "artifacts",
            "autotune",
            "config",
            "daemon",
            "recorder",
            "remote",
            "report",
            "service",
            "session",
            "session_io",
        ],
        must_not_depend_on: &[
            "actions::runner",
            "cli",
            "commands",
            "direct privileged mutation from remote requests",
        ],
    },
    DependencyMatrixEntry {
        subsystem: "daemon",
        may_depend_on: &[
            "actions",
            "autotune",
            "config",
            "daemon::capabilities",
            "daemon::health",
            "daemon::lifecycle",
            "daemon::policy",
            "daemon::privilege",
            "daemon::state",
            "daemon::store",
            "daemon::watchdog",
            "process_tree",
            "recorder",
            "session",
            "system",
        ],
        must_not_depend_on: &[
            "cli",
            "commands",
            "clap",
            "direct action mutation without DaemonPolicy",
        ],
    },
    DependencyMatrixEntry {
        subsystem: "autotune",
        may_depend_on: &[
            "actions",
            "autotune::candidate",
            "autotune::objective",
            "autotune::observation",
            "autotune::planner",
            "autotune::providers",
            "config",
            "daemon::policy",
            "focus",
            "process_tree",
            "recorder",
            "report",
            "system",
        ],
        must_not_depend_on: &[
            "cli",
            "commands",
            "direct sysfs mutation",
            "provider mutation",
        ],
    },
    DependencyMatrixEntry {
        subsystem: "actions",
        may_depend_on: &[
            "affinity",
            "audit",
            "daemon::policy",
            "hwmon",
            "irq_inspect",
            "process_tree",
            "procfs",
            "profile_restore",
            "system",
            "system_inventory",
            "task_class",
            "tasks",
            "topology",
        ],
        must_not_depend_on: &[
            "agent",
            "autotune::planner",
            "cli",
            "commands",
            "daemon::runtime",
            "recorder::live",
            "report",
        ],
    },
    DependencyMatrixEntry {
        subsystem: "report",
        may_depend_on: &[
            "autotune::report_overlay",
            "diagnosis",
            "metrics",
            "recorder::event_types",
            "runtime_slices",
            "session_io",
            "spike",
            "summary",
        ],
        must_not_depend_on: &[
            "actions::runner",
            "agent",
            "autotune::providers",
            "daemon::runtime",
            "recorder::live",
        ],
    },
    DependencyMatrixEntry {
        subsystem: "focus",
        may_depend_on: &[
            "community_rules",
            "config",
            "foreground",
            "metrics",
            "process_tree",
            "task_class",
        ],
        must_not_depend_on: &["actions", "agent", "daemon"],
    },
    DependencyMatrixEntry {
        subsystem: "events",
        may_depend_on: &[
            "metrics",
            "recorder::event_types",
            "runtime_slices",
            "stutter_common",
        ],
        must_not_depend_on: &[
            "actions",
            "agent",
            "autotune",
            "commands",
            "daemon",
            "recorder::live",
            "report",
        ],
    },
    DependencyMatrixEntry {
        subsystem: "recorder",
        may_depend_on: &[
            "config",
            "events",
            "foreground",
            "metrics",
            "runtime_slices",
            "session_io",
        ],
        must_not_depend_on: &[
            "actions::runner",
            "agent",
            "autotune::providers",
            "daemon::policy",
            "report",
        ],
    },
    DependencyMatrixEntry {
        subsystem: "config",
        may_depend_on: &[
            "config::effective",
            "config::layer",
            "config::merge",
            "config::model",
            "config::schema",
            "config::source",
            "config::types",
        ],
        must_not_depend_on: &[
            "actions", "agent", "autotune", "cli", "commands", "daemon", "recorder", "report",
        ],
    },
    DependencyMatrixEntry {
        subsystem: "process_tree",
        may_depend_on: &[
            "community_rules",
            "config",
            "procfs",
            "task_class",
            "task_filter",
            "tasks",
        ],
        must_not_depend_on: &[
            "actions::runner",
            "agent",
            "autotune::controller",
            "cli",
            "commands",
            "daemon::runtime",
            "recorder::live",
            "report",
        ],
    },
    DependencyMatrixEntry {
        subsystem: "system",
        may_depend_on: &[
            "config",
            "foreground",
            "hwmon",
            "irq_inspect",
            "kernel_event",
            "mangohud",
            "perf_counters",
            "process_tree",
            "psi",
            "sched_state",
            "scx",
            "stutter_common",
            "system_inventory",
            "topology",
        ],
        must_not_depend_on: &[
            "actions::runner",
            "agent",
            "autotune::providers",
            "cli",
            "commands",
            "daemon::runtime",
            "recorder::live",
            "report",
        ],
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
struct RustPathOccurrence {
    path: String,
    line_number: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ForbiddenRustPath {
    path: &'static str,
    boundary: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PathToken {
    kind: PathTokenKind,
    line_number: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum PathTokenKind {
    Ident(String),
    ColonColon,
    Punct(char),
}

fn crate_src_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn rust_files_under(path: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_rust_files(path, &mut files);
    files.sort();
    files
}

fn collect_rust_files(path: &Path, files: &mut Vec<PathBuf>) {
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };
    if metadata.is_file() {
        if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            files.push(path.to_path_buf());
        }
        return;
    }

    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        collect_rust_files(&entry.path(), files);
    }
}

fn assert_sources_do_not_reference_paths(files: &[PathBuf], forbidden: &[ForbiddenRustPath]) {
    let mut violations = Vec::new();

    for file in files {
        let source = fs::read_to_string(file).unwrap_or_default();
        for occurrence in rust_path_occurrences(&source) {
            for forbidden_path in forbidden {
                if rust_path_matches_forbidden(&occurrence.path, forbidden_path.path) {
                    violations.push(format_architecture_violation(
                        file,
                        &occurrence,
                        forbidden_path,
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "architecture boundary violations:\n{}",
        violations.join("\n")
    );
}

fn format_architecture_violation(
    file: &Path,
    occurrence: &RustPathOccurrence,
    forbidden: &ForbiddenRustPath,
) -> String {
    format!(
        "{}:{}: boundary '{}' forbids '{}', found '{}'",
        file.display(),
        occurrence.line_number,
        forbidden.boundary,
        forbidden.path,
        occurrence.path
    )
}

fn rust_path_matches_forbidden(path: &str, forbidden: &str) -> bool {
    if path == forbidden || path.starts_with(&format!("{forbidden}::")) {
        return true;
    }

    if let Some(stripped) = path.strip_prefix("crate::")
        && (stripped == forbidden || stripped.starts_with(&format!("{forbidden}::")))
    {
        return true;
    }

    if !forbidden.contains("::") {
        return path.split("::").any(|component| component == forbidden);
    }

    false
}

fn rust_path_occurrences(source: &str) -> Vec<RustPathOccurrence> {
    let sanitized = sanitize_rust_source(source);
    let tokens = lex_rust_path_tokens(&sanitized);
    let mut occurrences = Vec::new();

    collect_use_tree_paths(&tokens, &mut occurrences);
    collect_qualified_paths(&tokens, &mut occurrences);
    collect_bare_ident_paths(&tokens, &mut occurrences);
    dedupe_path_occurrences(occurrences)
}

fn dedupe_path_occurrences(occurrences: Vec<RustPathOccurrence>) -> Vec<RustPathOccurrence> {
    let mut deduped = Vec::new();
    for occurrence in occurrences {
        if !deduped.iter().any(|existing: &RustPathOccurrence| {
            existing.path == occurrence.path && existing.line_number == occurrence.line_number
        }) {
            deduped.push(occurrence);
        }
    }
    deduped
}

fn collect_bare_ident_paths(tokens: &[PathToken], occurrences: &mut Vec<RustPathOccurrence>) {
    for token in tokens {
        let PathTokenKind::Ident(ident) = &token.kind else {
            continue;
        };
        if !is_rust_keyword(ident) {
            occurrences.push(RustPathOccurrence {
                path: ident.clone(),
                line_number: token.line_number,
            });
        }
    }
}

fn collect_qualified_paths(tokens: &[PathToken], occurrences: &mut Vec<RustPathOccurrence>) {
    for index in 0..tokens.len() {
        if matches!(tokens[index].kind, PathTokenKind::ColonColon) {
            if let Some((path, line_number)) = parse_qualified_path_from(tokens, index + 1) {
                occurrences.push(RustPathOccurrence { path, line_number });
            }
            continue;
        }

        if !matches!(tokens[index].kind, PathTokenKind::Ident(_)) {
            continue;
        }

        if matches!(
            tokens.get(index + 1).map(|token| &token.kind),
            Some(PathTokenKind::ColonColon)
        ) && let Some((path, line_number)) = parse_qualified_path_from(tokens, index)
        {
            occurrences.push(RustPathOccurrence { path, line_number });
        }
    }
}

fn parse_qualified_path_from(tokens: &[PathToken], start: usize) -> Option<(String, usize)> {
    let first = tokens.get(start)?;
    let PathTokenKind::Ident(first_ident) = &first.kind else {
        return None;
    };

    let mut parts = vec![first_ident.clone()];
    let line_number = first.line_number;
    let mut cursor = start + 1;

    while matches!(
        tokens.get(cursor).map(|token| &token.kind),
        Some(PathTokenKind::ColonColon)
    ) {
        let Some(next) = tokens.get(cursor + 1) else {
            break;
        };
        let PathTokenKind::Ident(next_ident) = &next.kind else {
            break;
        };
        parts.push(next_ident.clone());
        cursor += 2;
    }

    (parts.len() > 1).then(|| (parts.join("::"), line_number))
}

fn collect_use_tree_paths(tokens: &[PathToken], occurrences: &mut Vec<RustPathOccurrence>) {
    let mut index = 0;
    while index < tokens.len() {
        if !token_is_ident(&tokens[index], "use") {
            index += 1;
            continue;
        }

        let mut cursor = index + 1;
        cursor = parse_use_tree(tokens, cursor, &[], occurrences);
        while cursor < tokens.len() && !matches!(tokens[cursor].kind, PathTokenKind::Punct(';')) {
            cursor += 1;
        }
        index = cursor.saturating_add(1);
    }
}

fn parse_use_group(
    tokens: &[PathToken],
    mut index: usize,
    prefix: &[String],
    occurrences: &mut Vec<RustPathOccurrence>,
) -> usize {
    while index < tokens.len() {
        match &tokens[index].kind {
            PathTokenKind::Punct('}') => return index + 1,
            PathTokenKind::Punct(',') => index += 1,
            _ => index = parse_use_tree(tokens, index, prefix, occurrences),
        }
    }
    index
}

fn parse_use_tree(
    tokens: &[PathToken],
    mut index: usize,
    prefix: &[String],
    occurrences: &mut Vec<RustPathOccurrence>,
) -> usize {
    let mut path_parts = prefix.to_vec();

    loop {
        let Some(token) = tokens.get(index) else {
            return index;
        };

        match &token.kind {
            PathTokenKind::Ident(ident) if ident == "self" => {
                if !path_parts.is_empty() {
                    occurrences.push(RustPathOccurrence {
                        path: path_parts.join("::"),
                        line_number: token.line_number,
                    });
                }
                return skip_use_alias(tokens, index + 1);
            }
            PathTokenKind::Ident(ident) => {
                let ident_line_number = token.line_number;
                path_parts.push(ident.clone());
                index += 1;

                if matches!(
                    tokens.get(index).map(|token| &token.kind),
                    Some(PathTokenKind::ColonColon)
                ) {
                    index += 1;
                    if matches!(
                        tokens.get(index).map(|token| &token.kind),
                        Some(PathTokenKind::Punct('{'))
                    ) {
                        return parse_use_group(tokens, index + 1, &path_parts, occurrences);
                    }
                    continue;
                }

                occurrences.push(RustPathOccurrence {
                    path: path_parts.join("::"),
                    line_number: ident_line_number,
                });
                return skip_use_alias(tokens, index);
            }
            PathTokenKind::Punct('*') => {
                path_parts.push("*".to_owned());
                occurrences.push(RustPathOccurrence {
                    path: path_parts.join("::"),
                    line_number: token.line_number,
                });
                return index + 1;
            }
            PathTokenKind::Punct('{') => {
                return parse_use_group(tokens, index + 1, &path_parts, occurrences);
            }
            _ => return index + 1,
        }
    }
}

fn skip_use_alias(tokens: &[PathToken], mut index: usize) -> usize {
    if token_is_ident_at(tokens, index, "as") {
        index += 1;
        if matches!(
            tokens.get(index).map(|token| &token.kind),
            Some(PathTokenKind::Ident(_))
        ) {
            index += 1;
        }
    }
    index
}

fn token_is_ident(token: &PathToken, expected: &str) -> bool {
    matches!(&token.kind, PathTokenKind::Ident(ident) if ident == expected)
}

fn token_is_ident_at(tokens: &[PathToken], index: usize, expected: &str) -> bool {
    tokens
        .get(index)
        .is_some_and(|token| token_is_ident(token, expected))
}

fn lex_rust_path_tokens(source: &str) -> Vec<PathToken> {
    let mut tokens = Vec::new();
    let mut chars = source.chars().peekable();
    let mut line_number = 1;

    while let Some(ch) = chars.next() {
        if ch == '\n' {
            line_number += 1;
            continue;
        }

        if is_ident_start(ch) {
            let mut ident = String::from(ch);
            while let Some(next) = chars.peek().copied() {
                if is_ident_continue(next) {
                    ident.push(next);
                    chars.next();
                } else {
                    break;
                }
            }
            tokens.push(PathToken {
                kind: PathTokenKind::Ident(ident),
                line_number,
            });
            continue;
        }

        if ch == ':' && chars.peek() == Some(&':') {
            chars.next();
            tokens.push(PathToken {
                kind: PathTokenKind::ColonColon,
                line_number,
            });
            continue;
        }

        if matches!(
            ch,
            '{' | '}' | '(' | ')' | '[' | ']' | ',' | ';' | '*' | '<' | '>'
        ) {
            tokens.push(PathToken {
                kind: PathTokenKind::Punct(ch),
                line_number,
            });
        }
    }

    tokens
}

fn sanitize_rust_source(source: &str) -> String {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum State {
        Normal,
        LineComment,
        BlockComment(usize),
        String { escaped: bool },
        RawString { hashes: usize },
    }

    let mut output = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    let mut state = State::Normal;

    while let Some(ch) = chars.next() {
        match state {
            State::Normal => {
                if ch == 'r'
                    && let Some(hashes) = try_start_raw_string(ch, &mut chars, &mut output)
                {
                    state = State::RawString { hashes };
                } else if ch == '/' && chars.peek() == Some(&'/') {
                    chars.next();
                    output.push(' ');
                    output.push(' ');
                    state = State::LineComment;
                } else if ch == '/' && chars.peek() == Some(&'*') {
                    chars.next();
                    output.push(' ');
                    output.push(' ');
                    state = State::BlockComment(1);
                } else if ch == '"' {
                    output.push(' ');
                    state = State::String { escaped: false };
                } else {
                    output.push(ch);
                }
            }
            State::LineComment => {
                if ch == '\n' {
                    output.push('\n');
                    state = State::Normal;
                } else {
                    output.push(' ');
                }
            }
            State::BlockComment(depth) => {
                if ch == '\n' {
                    output.push('\n');
                } else if ch == '/' && chars.peek() == Some(&'*') {
                    chars.next();
                    output.push(' ');
                    output.push(' ');
                    state = State::BlockComment(depth + 1);
                } else if ch == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    output.push(' ');
                    output.push(' ');
                    if depth == 1 {
                        state = State::Normal;
                    } else {
                        state = State::BlockComment(depth - 1);
                    }
                } else {
                    output.push(' ');
                }
            }
            State::String { escaped } => {
                if ch == '\n' {
                    output.push('\n');
                    state = State::Normal;
                } else {
                    output.push(' ');
                    if escaped {
                        state = State::String { escaped: false };
                    } else if ch == '\\' {
                        state = State::String { escaped: true };
                    } else if ch == '"' {
                        state = State::Normal;
                    }
                }
            }
            State::RawString { hashes } => {
                if ch == '\n' {
                    output.push('\n');
                } else if ch == '"' && raw_string_hashes_close(&mut chars, hashes, &mut output) {
                    output.push(' ');
                    state = State::Normal;
                } else {
                    output.push(' ');
                }
            }
        }
    }

    output
}

fn try_start_raw_string(
    first_char: char,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    output: &mut String,
) -> Option<usize> {
    if first_char != 'r' {
        return None;
    }

    let mut lookahead = chars.clone();
    let mut hashes = 0;
    while lookahead.peek() == Some(&'#') {
        hashes += 1;
        lookahead.next();
    }
    if lookahead.peek() != Some(&'"') {
        return None;
    }

    output.push(' ');
    for _ in 0..hashes {
        chars.next();
        output.push(' ');
    }
    chars.next();
    output.push(' ');
    Some(hashes)
}

fn raw_string_hashes_close(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    hashes: usize,
    output: &mut String,
) -> bool {
    let mut lookahead = chars.clone();
    for _ in 0..hashes {
        if lookahead.next() != Some('#') {
            return false;
        }
    }

    for _ in 0..hashes {
        chars.next();
        output.push(' ');
    }
    true
}

fn is_ident_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_ident_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn is_rust_keyword(ident: &str) -> bool {
    matches!(
        ident,
        "as" | "async"
            | "await"
            | "break"
            | "const"
            | "continue"
            | "crate"
            | "dyn"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
    )
}

fn dependency_matrix_entry(subsystem: &str) -> &'static DependencyMatrixEntry {
    ARCHITECTURE_DEPENDENCY_MATRIX
        .iter()
        .find(|entry| entry.subsystem == subsystem)
        .unwrap_or_else(|| panic!("missing architecture dependency matrix entry for {subsystem}"))
}

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

#[test]
fn dependency_matrix_covers_known_top_level_modules() {
    let mut matrix_subsystems = ARCHITECTURE_DEPENDENCY_MATRIX
        .iter()
        .map(|entry| entry.subsystem)
        .collect::<Vec<_>>();
    matrix_subsystems.sort_unstable();

    let mut unique_subsystems = matrix_subsystems.clone();
    unique_subsystems.dedup();
    assert_eq!(
        matrix_subsystems, unique_subsystems,
        "architecture dependency matrix contains duplicate subsystem entries"
    );

    let mut expected_subsystems = KNOWN_TOP_LEVEL_ARCHITECTURE_MODULES.to_vec();
    expected_subsystems.sort_unstable();

    assert_eq!(
        matrix_subsystems, expected_subsystems,
        "architecture dependency matrix must cover exactly the known top-level modules"
    );

    assert!(
        dependency_matrix_entry("cli")
            .may_depend_on
            .contains(&"commands"),
        "cli must be allowed to depend on commands"
    );

    let commands = dependency_matrix_entry("commands");
    assert!(
        commands.may_depend_on.contains(&"service"),
        "commands must be allowed to depend on service modules"
    );
    assert!(
        commands.may_depend_on.contains(&"daemon"),
        "commands must be allowed to dispatch to daemon application modules"
    );
    assert!(
        commands.may_depend_on.contains(&"actions"),
        "commands must be allowed to dispatch to action-backed application modules"
    );

    let agent = dependency_matrix_entry("agent");
    for dependency in ["daemon", "autotune", "recorder", "config", "remote"] {
        assert!(
            agent.may_depend_on.contains(&dependency),
            "agent must be allowed to depend on {dependency}"
        );
    }

    let daemon = dependency_matrix_entry("daemon");
    for dependency in ["daemon::policy", "daemon::state", "actions", "autotune"] {
        assert!(
            daemon.may_depend_on.contains(&dependency),
            "daemon must encode allowed dependency on {dependency}"
        );
    }

    let autotune = dependency_matrix_entry("autotune");
    for dependency in [
        "autotune::observation",
        "autotune::candidate",
        "autotune::providers",
        "autotune::objective",
    ] {
        assert!(
            autotune.may_depend_on.contains(&dependency),
            "autotune planning must encode allowed dependency on {dependency}"
        );
    }

    let actions = dependency_matrix_entry("actions");
    for dependency in ["affinity", "process_tree", "system"] {
        assert!(
            actions.may_depend_on.contains(&dependency),
            "actions must encode allowed low-level system dependency on {dependency}"
        );
    }

    let report = dependency_matrix_entry("report");
    for dependency in [
        "session_io",
        "summary",
        "diagnosis",
        "recorder::event_types",
    ] {
        assert!(
            report.may_depend_on.contains(&dependency),
            "report must encode allowed dependency on {dependency}"
        );
    }

    let focus = dependency_matrix_entry("focus");
    for dependency in ["process_tree", "config", "foreground", "community_rules"] {
        assert!(
            focus.may_depend_on.contains(&dependency),
            "focus must encode allowed dependency on {dependency}"
        );
    }
    for forbidden_dependency in ["actions", "daemon", "agent"] {
        assert!(
            focus.must_not_depend_on.contains(&forbidden_dependency),
            "focus must encode forbidden dependency on {forbidden_dependency}"
        );
    }
}

#[test]
fn actions_do_not_depend_on_cli_or_command_parsing() {
    let root = crate_src_root().join("actions");
    let files = rust_files_under(&root);

    assert_sources_do_not_reference_paths(
        &files,
        &[
            ForbiddenRustPath {
                path: "crate::cli",
                boundary: "actions must not depend on CLI parsing",
            },
            ForbiddenRustPath {
                path: "crate::commands",
                boundary: "actions must not depend on command parsing",
            },
            ForbiddenRustPath {
                path: "AppCommand",
                boundary: "actions must not depend on command DTOs",
            },
            ForbiddenRustPath {
                path: "clap",
                boundary: "actions must not depend on Clap parsing",
            },
        ],
    );
}

#[test]
fn daemon_internals_do_not_depend_on_cli_or_command_parsing() {
    let root = crate_src_root().join("daemon");
    let files = rust_files_under(&root);

    assert_sources_do_not_reference_paths(
        &files,
        &[
            ForbiddenRustPath {
                path: "crate::cli",
                boundary: "daemon internals must not depend on CLI parsing",
            },
            ForbiddenRustPath {
                path: "crate::commands",
                boundary: "daemon internals must not depend on command parsing",
            },
            ForbiddenRustPath {
                path: "AppCommand",
                boundary: "daemon internals must not depend on command DTOs",
            },
            ForbiddenRustPath {
                path: "clap",
                boundary: "daemon internals must not depend on Clap parsing",
            },
        ],
    );
}

#[test]
fn event_decode_module_does_not_depend_on_recording() {
    let files = vec![crate_src_root().join("events/decode.rs")];

    assert_sources_do_not_reference_paths(
        &files,
        &[
            ForbiddenRustPath {
                path: "crate::recorder",
                boundary: "events/decode.rs must not depend on recording",
            },
            ForbiddenRustPath {
                path: "recorder",
                boundary: "events/decode.rs must not depend on recording",
            },
            ForbiddenRustPath {
                path: "LiveRecorder",
                boundary: "events/decode.rs must not depend on live recording",
            },
        ],
    );
}

#[test]
fn policy_module_does_not_mutate_persistent_daemon_state() {
    let files = vec![crate_src_root().join("daemon/policy.rs")];

    assert_sources_do_not_reference_paths(
        &files,
        &[
            ForbiddenRustPath {
                path: "DaemonStateStore",
                boundary: "daemon policy must not mutate persistent daemon state",
            },
            ForbiddenRustPath {
                path: "DaemonStateSnapshotWriter",
                boundary: "daemon policy must not mutate persistent daemon state",
            },
            ForbiddenRustPath {
                path: "load_daemon_state",
                boundary: "daemon policy must not load persistent daemon state",
            },
            ForbiddenRustPath {
                path: "default_daemon_state_snapshot_path",
                boundary: "daemon policy must not know persistent daemon state paths",
            },
        ],
    );
}
