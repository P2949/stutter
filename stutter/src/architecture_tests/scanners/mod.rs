//! Rust source scanners used by architecture tests; this module owns parsing/walking helpers, not policy tables.

use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::architecture_tests) struct RustPathOccurrence {
    pub(in crate::architecture_tests) path: String,
    pub(in crate::architecture_tests) line_number: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::architecture_tests) struct ForbiddenRustPath {
    pub(in crate::architecture_tests) path: &'static str,
    pub(in crate::architecture_tests) boundary: &'static str,
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

pub(in crate::architecture_tests) fn rust_files_under(path: &Path) -> Vec<PathBuf> {
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

pub(in crate::architecture_tests) fn assert_sources_do_not_reference_paths(
    files: &[PathBuf],
    forbidden: &[ForbiddenRustPath],
) {
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

pub(in crate::architecture_tests) fn format_architecture_violation(
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

pub(in crate::architecture_tests) fn rust_path_occurrences(
    source: &str,
) -> Vec<RustPathOccurrence> {
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

pub(in crate::architecture_tests) fn production_code_lines_outside_cfg_test_modules(
    source: &str,
) -> Vec<(usize, &str)> {
    if source
        .lines()
        .map(str::trim)
        .take_while(|line| line.is_empty() || line.starts_with("//") || line.starts_with("#!["))
        .any(|line| line == "#![cfg(test)]")
    {
        return Vec::new();
    }

    let mut lines = Vec::new();
    let mut cfg_test_pending = false;
    let mut skipped_test_module_brace_depth: Option<isize> = None;

    for (zero_based_line_number, line) in source.lines().enumerate() {
        let line_number = zero_based_line_number + 1;
        let trimmed = line.trim_start();

        if let Some(depth) = skipped_test_module_brace_depth {
            let next_depth = depth + brace_delta(line);
            if next_depth <= 0 {
                skipped_test_module_brace_depth = None;
            } else {
                skipped_test_module_brace_depth = Some(next_depth);
            }
            continue;
        }

        if trimmed.starts_with("#[cfg(test)]") {
            cfg_test_pending = true;
            if trimmed.contains("mod tests") && trimmed.contains('{') {
                let depth = brace_delta(line);
                if depth > 0 {
                    skipped_test_module_brace_depth = Some(depth);
                }
            }
            continue;
        }

        if cfg_test_pending && trimmed.contains('{') {
            cfg_test_pending = false;
            let depth = brace_delta(line);
            if depth > 0 {
                skipped_test_module_brace_depth = Some(depth);
            }
            continue;
        }

        if cfg_test_pending
            && !trimmed.is_empty()
            && !trimmed.starts_with('#')
            && !trimmed.starts_with("//")
        {
            cfg_test_pending = false;
            continue;
        }

        lines.push((line_number, line));
    }

    lines
}

fn brace_delta(line: &str) -> isize {
    line.chars().filter(|ch| *ch == '{').count() as isize
        - line.chars().filter(|ch| *ch == '}').count() as isize
}

mod rust_files;

mod raw_source;
