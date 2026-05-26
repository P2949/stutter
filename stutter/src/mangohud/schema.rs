use std::{io::BufRead, path::Path};

use super::*;

const FRAMETIME_HEADERS: &[&str] = &[
    "frametime",
    "frametime_ms",
    "frame_time",
    "frame_time_ms",
    "frame time",
    "frame time ms",
];

const ELAPSED_HEADERS: &[&str] = &[
    "elapsed_ms",
    "time_ms",
    "ms",
    "time",
    "elapsed",
    "elapsed_ns",
    "time_ns",
    "ns",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ElapsedUnit {
    Milliseconds,
    Nanoseconds,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct MangoHudCsvSchema {
    pub(super) frametime_idx: usize,
    pub(super) elapsed_idx: Option<usize>,
    pub(super) elapsed_unit: ElapsedUnit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct MangoHudCsvLayout {
    pub(super) schema: MangoHudCsvSchema,
    pub(super) data_start_offset: u64,
}

pub(super) fn detect_layout(path: &Path) -> anyhow::Result<MangoHudCsvLayout> {
    try_detect_layout(path)?.with_context(|| {
        format!(
            "MangoHud CSV did not contain a recognized frame header: {}",
            path.display()
        )
    })
}

pub(super) fn try_detect_layout(path: &Path) -> std::io::Result<Option<MangoHudCsvLayout>> {
    let file = fs::File::open(path)?;
    detect_layout_from_reader(std::io::BufReader::new(file))
}

pub(super) fn detect_layout_from_reader<R: BufRead>(
    mut reader: R,
) -> std::io::Result<Option<MangoHudCsvLayout>> {
    let mut offset = 0_u64;

    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            return Ok(None);
        }

        let next_offset = offset + n as u64;
        let headers = split_csv_line(&line);
        if let Some(schema) = schema_from_headers(&headers) {
            return Ok(Some(MangoHudCsvLayout {
                schema,
                data_start_offset: next_offset,
            }));
        }

        offset = next_offset;
    }
}

pub(super) fn schema_from_headers(headers: &[String]) -> Option<MangoHudCsvSchema> {
    let frametime_idx = find_header(headers, FRAMETIME_HEADERS)?;
    let elapsed_idx = find_header(headers, ELAPSED_HEADERS);
    let elapsed_unit = elapsed_idx
        .and_then(|idx| headers.get(idx))
        .map(|header| elapsed_unit_for_header(header))
        .unwrap_or(ElapsedUnit::Milliseconds);

    Some(MangoHudCsvSchema {
        frametime_idx,
        elapsed_idx,
        elapsed_unit,
    })
}

pub(super) fn elapsed_value_to_ms(raw: f64, unit: ElapsedUnit) -> u64 {
    let raw = raw.max(0.0);
    match unit {
        ElapsedUnit::Milliseconds => raw as u64,
        ElapsedUnit::Nanoseconds => (raw / 1_000_000.0) as u64,
    }
}

pub(super) fn split_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut chars = line.chars().peekable();
    let mut in_quotes = false;

    while let Some(ch) = chars.next() {
        match ch {
            '"' if in_quotes && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                fields.push(field.trim().to_owned());
                field.clear();
            }
            _ => field.push(ch),
        }
    }

    fields.push(field.trim().to_owned());
    fields
}

fn elapsed_unit_for_header(header: &str) -> ElapsedUnit {
    match header.trim().to_ascii_lowercase().as_str() {
        "elapsed" | "elapsed_ns" | "time_ns" | "ns" => ElapsedUnit::Nanoseconds,
        _ => ElapsedUnit::Milliseconds,
    }
}

fn find_header(headers: &[String], candidates: &[&str]) -> Option<usize> {
    headers.iter().position(|header| {
        let normalized = header.trim().to_ascii_lowercase();
        candidates.iter().any(|candidate| normalized == *candidate)
    })
}
