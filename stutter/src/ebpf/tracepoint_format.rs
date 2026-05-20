#![allow(dead_code)] // Transitional eBPF split: tracepoint validation migrates from ebpf_loader.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::Context;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TracepointField {
    pub(crate) name: String,
    pub(crate) offset: u32,
    pub(crate) size: u32,
    pub(crate) signed: bool,
    pub(crate) declaration: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TracepointFormat {
    pub(crate) path: PathBuf,
    pub(crate) fields: BTreeMap<String, TracepointField>,
}

pub(crate) fn tracepoint_field_has_offset_and_size(
    format: &TracepointFormat,
    field_name: &str,
    expected_offset: u32,
    min_size: u32,
) -> bool {
    let Some(field) = format.fields.get(field_name) else {
        log::warn!(
            "tracepoint_required_field_missing path={} field={} expected_offset={} required_size={}",
            format.path.display(),
            field_name,
            expected_offset,
            min_size
        );
        return false;
    };

    if field.offset != expected_offset || field.size < min_size {
        log::warn!(
            "tracepoint_required_field_invalid path={} field={} offset={} expected_offset={} size={} required_size={}",
            format.path.display(),
            field.name,
            field.offset,
            expected_offset,
            field.size,
            min_size
        );
        return false;
    }

    true
}

pub(crate) fn validated_tracepoint_field_offset(
    format: &TracepointFormat,
    field_name: &str,
    min_size: u32,
    read_type: &str,
) -> Option<u32> {
    let Some(field) = format.fields.get(field_name) else {
        log::warn!(
            "tracepoint_field_missing path={} field={} read_type={}",
            format.path.display(),
            field_name,
            read_type
        );
        return None;
    };

    if field.size < min_size {
        log::warn!(
            "tracepoint_field_too_small path={} field={} offset={} size={} required_size={} read_type={}",
            format.path.display(),
            field.name,
            field.offset,
            field.size,
            min_size,
            read_type
        );
        return None;
    }

    Some(field.offset)
}

pub(crate) fn parse_tracepoint_format_at(path: &Path) -> anyhow::Result<TracepointFormat> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("failed to read tracepoint format {}", path.display()))?;
    Ok(parse_tracepoint_format(path.to_path_buf(), &contents))
}

pub(crate) fn parse_tracepoint_format(path: PathBuf, contents: &str) -> TracepointFormat {
    let fields = contents
        .lines()
        .filter_map(parse_tracepoint_field_line)
        .map(|field| (field.name.clone(), field))
        .collect();

    TracepointFormat { path, fields }
}

fn parse_tracepoint_field_line(line: &str) -> Option<TracepointField> {
    let mut name = None;
    let mut offset = None;
    let mut size = None;
    let mut signed = None;

    for part in line.split(';') {
        let part = part.trim();
        if let Some(declaration) = part.strip_prefix("field:") {
            name = parse_tracepoint_field_name(declaration);
        } else if let Some(value) = part.strip_prefix("offset:") {
            offset = value.trim().parse::<u32>().ok();
        } else if let Some(value) = part.strip_prefix("size:") {
            size = value.trim().parse::<u32>().ok();
        } else if let Some(value) = part.strip_prefix("signed:") {
            signed = Some(value.trim() != "0");
        }
    }

    Some(TracepointField {
        name: name?,
        offset: offset?,
        size: size?,
        signed: signed.unwrap_or(false),
        declaration: line.trim().to_owned(),
    })
}

fn parse_tracepoint_field_name(declaration: &str) -> Option<String> {
    let token = declaration.split_whitespace().last()?;
    let token = token.trim_start_matches('*');
    let token = token.split('[').next().unwrap_or(token);
    let token = token.trim();

    if token.is_empty() {
        None
    } else {
        Some(token.to_owned())
    }
}

pub(crate) fn validate_optional_tracepoint_format_at(
    path: &Path,
    name: &str,
    expected_offsets: &[(&str, usize)],
    warn_on_missing: bool,
) -> anyhow::Result<bool> {
    if !path.exists() {
        if warn_on_missing {
            log::warn!(
                "optional tracepoint format missing: {}; continuing without {name}",
                path.display()
            );
        }
        return Ok(false);
    }

    validate_tracepoint_format_at_named(path, name, expected_offsets)?;
    Ok(true)
}

pub(crate) fn validate_tracepoint_format_at(
    path: &Path,
    expected_offsets: &[(&str, usize)],
) -> anyhow::Result<()> {
    let tracepoint_name = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .unwrap_or("tracepoint");

    validate_tracepoint_format_at_named(path, tracepoint_name, expected_offsets)
}

pub(crate) fn validate_tracepoint_format_at_named(
    path: &Path,
    tracepoint_name: &str,
    expected_offsets: &[(&str, usize)],
) -> anyhow::Result<()> {
    let format = fs::read_to_string(path)
        .with_context(|| format!("failed to read tracepoint format {}", path.display()))?;

    validate_tracepoint_format_named(tracepoint_name, &format, expected_offsets).with_context(
        || {
            format!(
                "{tracepoint_name} tracepoint format {} did not match the eBPF program assumptions",
                path.display(),
            )
        },
    )
}

pub(crate) fn require_tracepoint_field(
    format_path: &Path,
    field_name: &str,
) -> anyhow::Result<u32> {
    let contents = fs::read_to_string(format_path)
        .with_context(|| format!("failed to read tracepoint format {}", format_path.display()))?;

    parse_tracepoint_field_offset(&contents, field_name).with_context(|| {
        format!(
            "tracepoint format {} is missing required field {:?}",
            format_path.display(),
            field_name
        )
    })
}

pub(crate) fn parse_tracepoint_field_offset(
    format_content: &str,
    field_name: &str,
) -> anyhow::Result<u32> {
    let format = parse_tracepoint_format(PathBuf::from("tracepoint"), format_content);
    format
        .fields
        .get(field_name)
        .map(|f| f.offset)
        .ok_or_else(|| anyhow::anyhow!("missing tracepoint field {:?}", field_name))
}

#[cfg(test)]
pub(crate) fn validate_tracepoint_format(
    format: &str,
    expected_offsets: &[(&str, usize)],
) -> anyhow::Result<()> {
    validate_tracepoint_format_named("tracepoint", format, expected_offsets)
}

pub(crate) fn validate_tracepoint_format_named(
    tracepoint_name: &str,
    format_content: &str,
    expected_offsets: &[(&str, usize)],
) -> anyhow::Result<()> {
    let format = parse_tracepoint_format(PathBuf::from(tracepoint_name), format_content);
    let fields = &format.fields;

    for &(field_name, expected_offset) in expected_offsets {
        let Some(field) = fields.get(field_name) else {
            return Err(tracepoint_missing_field_error(
                tracepoint_name,
                field_name,
                fields,
            ));
        };

        if field.offset as usize != expected_offset {
            return Err(tracepoint_offset_mismatch_error(
                tracepoint_name,
                field_name,
                expected_offset,
                field,
            ));
        }
    }

    Ok(())
}

fn tracepoint_offset_mismatch_error(
    tracepoint_name: &str,
    field_name: &str,
    expected: usize,
    field: &TracepointField,
) -> anyhow::Error {
    anyhow::anyhow!(
        "{} tracepoint layout mismatch for field `{}`: expected offset {}, got {}. Parsed declaration: `{}`{}",
        tracepoint_name,
        field_name,
        expected,
        field.offset,
        field.declaration,
        tracepoint_layout_hint(tracepoint_name, field_name),
    )
}

fn tracepoint_missing_field_error(
    tracepoint_name: &str,
    field_name: &str,
    fields: &BTreeMap<String, TracepointField>,
) -> anyhow::Error {
    let available = fields.keys().cloned().collect::<Vec<_>>().join(", ");

    anyhow::anyhow!(
        "{} tracepoint missing expected field `{}`. Available parsed fields: [{}].{}",
        tracepoint_name,
        field_name,
        available,
        tracepoint_layout_hint(tracepoint_name, field_name),
    )
}

fn tracepoint_layout_hint(tracepoint_name: &str, field_name: &str) -> &'static str {
    if tracepoint_name == "sched_switch"
        && matches!(
            field_name,
            "prev_state" | "next_comm" | "next_pid" | "next_prio"
        )
    {
        " Hint: `sched_switch` layout differs from stutter's eBPF read offsets. A common cause is a different `prev_state` field type/size, which shifts later fields such as `next_comm`, `next_pid`, and `next_prio`. stutter rejects this layout to avoid reading the wrong tracepoint bytes."
    } else {
        " Hint: the running kernel tracepoint format does not match stutter's compiled eBPF read offsets. stutter rejects this layout to avoid mis-decoding tracepoint data."
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ExpectedTracepointField {
    pub name: &'static str,
    pub offset: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ExpectedTracepointFormat {
    pub name: &'static str,
    pub fields: &'static [ExpectedTracepointField],
}
