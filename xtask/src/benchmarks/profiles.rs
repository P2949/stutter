//! `perf script` samples and the CSV profile tables of Intel VTune, AMD uProf and LIKWID as typed
//! diagnostic scores. Like every importer here they are observations of the host: not an
//! application outcome, no direction, no authority, and an unreadable input is an error.
use std::collections::BTreeMap;

use anyhow::ensure;

use super::{
    MAX_SAMPLES,
    scores::{ImportedScores, Score, finite},
};

fn diagnostic(tool: &str, label: String, value: f64, unit: &str) -> Score {
    Score {
        tool: tool.to_owned(),
        label,
        value,
        unit: unit.to_owned(),
        higher_is_better: None,
    }
}

fn done(tool: &str, scores: Vec<Score>) -> anyhow::Result<ImportedScores> {
    ensure!(!scores.is_empty(), "no {tool} values found");
    ensure!(scores.len() <= MAX_SAMPLES, "too many {tool} values");
    Ok(ImportedScores {
        tool: tool.to_owned(),
        scores,
        application_proof: false,
    })
}

/// `perf script` default rows: `comm pid [cpu] time: period event: addr symbol (dso)`. The score
/// is, per event, the number of samples and their summed period, and per command the share of the
/// samples. A row that does not parse is skipped, never counted.
pub fn import_perf_script(text: &str) -> anyhow::Result<ImportedScores> {
    let mut per_event: BTreeMap<String, (u64, f64)> = BTreeMap::new();
    let mut per_comm: BTreeMap<String, u64> = BTreeMap::new();
    let mut total = 0u64;
    for line in text.lines() {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        // comm pid [cpu] time: period event: ...
        let Some(cpu_at) = tokens
            .iter()
            .position(|t| t.starts_with('[') && t.ends_with(']'))
        else {
            continue;
        };
        if cpu_at < 2 || tokens.len() < cpu_at + 4 {
            continue;
        }
        let time = tokens[cpu_at + 1];
        let (Some(true), Some(period), Some(event)) = (
            time.strip_suffix(':').map(|t| finite(t).is_some()),
            finite(tokens[cpu_at + 2]),
            tokens[cpu_at + 3].strip_suffix(':'),
        ) else {
            continue;
        };
        if period < 0.0 {
            continue;
        }
        let comm = tokens[..cpu_at - 1].join(" ");
        let entry = per_event.entry(event.to_owned()).or_default();
        entry.0 += 1;
        entry.1 += period;
        *per_comm.entry(comm).or_default() += 1;
        total += 1;
        ensure!(
            per_event.len() <= MAX_SAMPLES && per_comm.len() <= MAX_SAMPLES,
            "too many rows"
        );
    }
    let mut scores = Vec::new();
    for (event, (samples, period)) in &per_event {
        scores.push(diagnostic(
            "perf-script",
            format!("{event}.samples"),
            *samples as f64,
            "samples",
        ));
        scores.push(diagnostic(
            "perf-script",
            format!("{event}.period"),
            *period,
            "events",
        ));
    }
    for (comm, samples) in &per_comm {
        scores.push(diagnostic(
            "perf-script",
            format!("{comm}.share"),
            *samples as f64 / total as f64 * 100.0,
            "% of samples",
        ));
    }
    done("perf-script", scores)
}

/// Which tool wrote a CSV profile table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileTool {
    Vtune,
    Uprof,
    Likwid,
}

impl ProfileTool {
    fn name(self) -> &'static str {
        match self {
            Self::Vtune => "vtune",
            Self::Uprof => "uprof",
            Self::Likwid => "likwid",
        }
    }
}

fn split_csv(line: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in line.chars() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => cells.push(std::mem::take(&mut current).trim().to_owned()),
            _ => current.push(c),
        }
    }
    cells.push(current.trim().to_owned());
    cells
}

/// A header-led CSV table: the first column names the row (function, metric, region), every other
/// numeric cell becomes `row.column`. The header is the first row with several columns that is
/// followed by rows of the same width; non-numeric cells and short rows are skipped.
pub fn import_profile_table(tool: ProfileTool, text: &str) -> anyhow::Result<ImportedScores> {
    let name = tool.name();
    let rows: Vec<Vec<String>> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(split_csv)
        .collect();
    let header_at = rows.iter().enumerate().position(|(i, row)| {
        row.len() >= 2
            && row[1..]
                .iter()
                .all(|c| !c.is_empty() && finite(c).is_none())
            && rows.get(i + 1).is_some_and(|next| next.len() == row.len())
    });
    let mut scores = Vec::new();
    if let Some(at) = header_at {
        let header = &rows[at];
        for row in rows[at + 1..].iter().filter(|r| r.len() == header.len()) {
            for (column, cell) in header[1..].iter().zip(&row[1..]) {
                if let Some(value) = finite(cell) {
                    ensure!(scores.len() < MAX_SAMPLES, "too many rows");
                    scores.push(diagnostic(name, format!("{}.{column}", row[0]), value, ""));
                }
            }
        }
    }
    done(name, scores)
}
