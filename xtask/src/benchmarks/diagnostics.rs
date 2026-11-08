//! iostat, pidstat, numastat, blkparse, tuna and cyclictest text as typed diagnostic scores.
//!
//! These are observations of the host, never application outcomes and never authority: every
//! import is an `ImportedScores` whose `application_proof` is false, no column is given a "better"
//! direction, and an unreadable table is an error, not an empty success.
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

/// `iostat -x`: one score per device and column under the `Device` header.
pub fn import_iostat(text: &str) -> anyhow::Result<ImportedScores> {
    let mut columns: Vec<&str> = Vec::new();
    let mut scores = Vec::new();
    for line in text.lines() {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        match tokens.first() {
            Some(first) if first.trim_end_matches(':') == "Device" => {
                columns = tokens[1..].to_vec();
            }
            Some(device) if !columns.is_empty() && tokens.len() == columns.len() + 1 => {
                for (column, cell) in columns.iter().zip(&tokens[1..]) {
                    if let Some(value) = finite(cell) {
                        scores.push(diagnostic(
                            "iostat",
                            format!("{device}.{column}"),
                            value,
                            "",
                        ));
                    }
                }
            }
            _ => columns.clear(),
        }
    }
    done("iostat", scores)
}

/// `pidstat`: one score per process and numeric column. Columns are aligned from the right so the
/// time-of-day prefix (with or without AM/PM) never shifts them.
pub fn import_pidstat(text: &str) -> anyhow::Result<ImportedScores> {
    let mut header: Vec<&str> = Vec::new();
    let mut scores = Vec::new();
    for line in text.lines() {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        if tokens.contains(&"PID") && tokens.contains(&"Command") {
            header = tokens;
            continue;
        }
        if header.is_empty() || tokens.len() < 3 {
            continue;
        }
        let first = header.iter().position(|t| *t == "PID").unwrap_or(0);
        let command = header.len() - 1;
        if tokens.len() < header.len() - first {
            header.clear();
            continue;
        }
        let row = &tokens[tokens.len() - (header.len() - first)..];
        let (Some(pid), Some(name)) = (row.first(), row.last()) else {
            continue;
        };
        for (offset, cell) in row.iter().enumerate().skip(1).take(command - first - 1) {
            if let Some(value) = finite(cell) {
                scores.push(diagnostic(
                    "pidstat",
                    format!("{name}[{pid}].{}", header[first + offset]),
                    value,
                    "",
                ));
            }
        }
    }
    done("pidstat", scores)
}

/// `numastat`: one score per row and node under the `node*` header.
pub fn import_numastat(text: &str) -> anyhow::Result<ImportedScores> {
    let mut nodes: Vec<&str> = Vec::new();
    let mut scores = Vec::new();
    for line in text.lines() {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        if !tokens.is_empty()
            && tokens
                .iter()
                .all(|t| t.starts_with("node") || *t == "Total")
        {
            nodes = tokens;
            continue;
        }
        if nodes.is_empty() {
            continue;
        }
        let label_len = tokens.iter().take_while(|t| finite(t).is_none()).count();
        if label_len == 0 || tokens.len() != label_len + nodes.len() {
            continue;
        }
        let row = tokens[..label_len].join("_");
        for (node, cell) in nodes.iter().zip(&tokens[label_len..]) {
            if let Some(value) = finite(cell) {
                scores.push(diagnostic("numastat", format!("{row}.{node}"), value, ""));
            }
        }
    }
    done("numastat", scores)
}

/// `blkparse` summary: completed read and write counts and the event total.
pub fn import_blkparse(text: &str) -> anyhow::Result<ImportedScores> {
    let count = |text: &str| -> Option<f64> {
        finite(text.split([',', ' ']).find(|part| !part.is_empty())?)
    };
    let mut scores = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Reads Completed:")
            && let Some(value) = count(rest)
        {
            scores.push(diagnostic(
                "blkparse",
                "reads_completed".to_owned(),
                value,
                "ios",
            ));
        }
        if let Some(rest) = line.strip_prefix("Writes Completed:")
            && let Some(value) = count(rest)
        {
            scores.push(diagnostic(
                "blkparse",
                "writes_completed".to_owned(),
                value,
                "ios",
            ));
        }
        if let Some(rest) = line.strip_prefix("Events (")
            && let Some((_, after)) = rest.split_once("):")
            && let Some(value) = count(after)
        {
            scores.push(diagnostic(
                "blkparse",
                "events".to_owned(),
                value,
                "entries",
            ));
        }
    }
    done("blkparse", scores)
}

/// `tuna --show_threads`: rtprio and context-switch counts per thread. The affinity column is a
/// description, not a number, and is not imported.
pub fn import_tuna(text: &str) -> anyhow::Result<ImportedScores> {
    let mut in_table = false;
    let mut scores = Vec::new();
    for line in text.lines() {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        if tokens.contains(&"pid") && tokens.contains(&"cmd") {
            in_table = true;
            continue;
        }
        if !in_table || tokens.len() < 7 {
            continue;
        }
        let Some(pid) = tokens[0].parse::<u32>().ok() else {
            continue;
        };
        let name = tokens[6];
        for (label, cell) in [
            ("rtprio", tokens[2]),
            ("voluntary_ctxt_switches", tokens[4]),
            ("nonvoluntary_ctxt_switches", tokens[5]),
        ] {
            if let Some(value) = finite(cell) {
                scores.push(diagnostic(
                    "tuna",
                    format!("{name}[{pid}].{label}"),
                    value,
                    "",
                ));
            }
        }
    }
    done("tuna", scores)
}

/// `cyclictest -q` summary lines: `T: 0 (123) P:95 I:1000 C: 100 Min: 3 Act: 5 Avg: 4 Max: 21`.
pub fn import_cyclictest(text: &str) -> anyhow::Result<ImportedScores> {
    let mut scores = Vec::new();
    for line in text.lines().filter(|l| l.trim_start().starts_with("T:")) {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        let thread = tokens.get(1).copied().unwrap_or("?");
        for window in tokens.windows(2) {
            let label = match window[0] {
                "Min:" => "min",
                "Avg:" => "avg",
                "Max:" => "max",
                _ => continue,
            };
            if let Some(value) = finite(window[1]) {
                scores.push(Score {
                    tool: "cyclictest".to_owned(),
                    label: format!("t{thread}.{label}"),
                    value,
                    unit: "us".to_owned(),
                    higher_is_better: Some(false),
                });
            }
        }
    }
    done("cyclictest", scores)
}
