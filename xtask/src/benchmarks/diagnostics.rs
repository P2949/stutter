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

/// A bucket bound such as `0`, `16`, `1K`, `2M` as a number.
fn bound(text: &str) -> Option<f64> {
    let text = text.trim();
    let (digits, scale) = match text.chars().last()? {
        'K' => (&text[..text.len() - 1], 1e3),
        'M' => (&text[..text.len() - 1], 1e6),
        'G' => (&text[..text.len() - 1], 1e9),
        _ => (text, 1.0),
    };
    digits
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v >= 0.0)
        .map(|v| v * scale)
}

/// Scores for a histogram given as `(bucket lower bound, count)` rows: each bucket's count, the
/// total, and the lower bound of the bucket that holds the 99th percentile. A bucket is a range, so
/// the percentile is reported as that bucket's lower bound and never as a point value.
fn histogram_scores(tool: &str, rows: &[(f64, f64)]) -> anyhow::Result<ImportedScores> {
    let total: f64 = rows.iter().map(|(_, count)| count).sum();
    ensure!(total > 0.0, "the {tool} histogram has no counts");
    let mut scores: Vec<Score> = rows
        .iter()
        .map(|(lower, count)| diagnostic(tool, format!("bucket_{lower}"), *count, "count"))
        .collect();
    scores.push(diagnostic(tool, "total".to_owned(), total, "count"));
    let mut seen = 0.0;
    for (lower, count) in rows {
        seen += count;
        if seen >= total * 0.99 {
            scores.push(diagnostic(
                tool,
                "p99_bucket_lower_bound".to_owned(),
                *lower,
                "",
            ));
            break;
        }
    }
    done(tool, scores)
}

/// `bpftrace` `hist()` output: rows such as `[2, 4)    7 |@@@|` or `[0]   12 |@@|`.
pub fn import_bpftrace_hist(text: &str) -> anyhow::Result<ImportedScores> {
    let mut rows = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix('[') else {
            continue;
        };
        let Some((range, tail)) = rest.split_once([')', ']']) else {
            continue;
        };
        let Some(lower) = range.split(',').next().and_then(bound) else {
            continue;
        };
        let Some(count) = tail
            .split_whitespace()
            .next()
            .and_then(|c| c.parse::<f64>().ok())
        else {
            continue;
        };
        ensure!(rows.len() < MAX_SAMPLES, "too many buckets");
        rows.push((lower, count));
    }
    histogram_scores("bpftrace", &rows)
}

/// BCC `runqlat`/`biolatency` histograms: rows such as `   4 -> 7   : 12   |***|`.
pub fn import_bcc_hist(text: &str) -> anyhow::Result<ImportedScores> {
    let mut rows = Vec::new();
    for line in text.lines() {
        let Some((range, tail)) = line.split_once(':') else {
            continue;
        };
        let Some((lower, _upper)) = range.split_once("->") else {
            continue;
        };
        let (Some(lower), Some(count)) = (
            bound(lower),
            tail.split_whitespace()
                .next()
                .and_then(|c| c.parse::<f64>().ok()),
        ) else {
            continue;
        };
        ensure!(rows.len() < MAX_SAMPLES, "too many buckets");
        rows.push((lower, count));
    }
    histogram_scores("bcc", &rows)
}

/// `trace-cmd report` or ftrace text: a count per event name, and the events the tracer dropped.
pub fn import_trace_cmd(text: &str) -> anyhow::Result<ImportedScores> {
    let mut counts: std::collections::BTreeMap<String, f64> = std::collections::BTreeMap::new();
    let mut dropped = 0.0;
    for line in text.lines() {
        if let Some(rest) = line
            .split("EVENTS DROPPED")
            .next()
            .filter(|_| line.contains("EVENTS DROPPED"))
        {
            if let Some(n) = rest
                .rsplit('[')
                .next()
                .and_then(|n| n.trim().parse::<f64>().ok())
            {
                dropped += n;
            }
            continue;
        }
        // `task-1234 [002] 1234.567890: sched_switch: prev_comm=...`
        let Some((head, tail)) = line.split_once(": ") else {
            continue;
        };
        if !head.contains('[') || !head.contains(']') {
            continue;
        }
        let Some(event) = tail
            .split(':')
            .next()
            .map(str::trim)
            .filter(|e| !e.is_empty() && !e.contains(' '))
        else {
            continue;
        };
        *counts.entry(event.to_owned()).or_default() += 1.0;
        ensure!(counts.len() <= MAX_SAMPLES, "too many event names");
    }
    let mut scores: Vec<Score> = counts
        .into_iter()
        .map(|(event, n)| diagnostic("trace-cmd", format!("events.{event}"), n, "events"))
        .collect();
    if !scores.is_empty() {
        scores.push(diagnostic(
            "trace-cmd",
            "events_dropped".to_owned(),
            dropped,
            "events",
        ));
    }
    done("trace-cmd", scores)
}

/// Least share of the interval an event must have been counting for its value to be used. Below
/// this the kernel multiplexed the counter and the number is an extrapolation, not a measurement.
const MIN_PERF_RUNNING_PERCENT: f64 = 90.0;

/// `perf stat -x,` CSV: `value,unit,event,run_time_ns,percent_running,...`. An event that was not
/// counted, not supported, or counted for less than the running floor gives no value, only its
/// `.running_percent` when that is known; a value is never extrapolated or filled in.
pub fn import_perf_stat(text: &str) -> anyhow::Result<ImportedScores> {
    let mut scores = Vec::new();
    for line in text.lines() {
        if line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split(',').collect();
        if fields.len() < 5 {
            continue;
        }
        let (value, event) = (fields[0].trim(), fields[2].trim());
        if event.is_empty() {
            continue;
        }
        let running = fields[4]
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|p| p.is_finite());
        if let Some(percent) = running {
            scores.push(diagnostic(
                "perf-stat",
                format!("{event}.running_percent"),
                percent,
                "%",
            ));
        }
        let Some(count) = value
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite() && *v >= 0.0)
        else {
            continue;
        };
        if running.is_some_and(|percent| percent >= MIN_PERF_RUNNING_PERCENT) {
            ensure!(scores.len() < MAX_SAMPLES, "too many events");
            scores.push(diagnostic("perf-stat", event.to_owned(), count, "count"));
        }
    }
    done("perf-stat", scores)
}

/// `perf report --stdio` (or `--no-children`) rows: `  12.34%  command  shared_object  [.] symbol`.
/// Each row becomes the share of samples for that symbol. A share is a proportion of the recorded
/// samples, not a time or a count, and `perf` percentages that do not parse are skipped.
pub fn import_perf_report(text: &str) -> anyhow::Result<ImportedScores> {
    let mut scores = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        let Some((percent, rest)) = line.split_once('%') else {
            continue;
        };
        let Some(share) = percent
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite() && (0.0..=100.0).contains(v))
        else {
            continue;
        };
        // After the percentage: command, shared object, then `[.]`/`[k]` and the symbol.
        let Some((head, symbol)) = rest.split_once("] ") else {
            continue;
        };
        let mut head = head.split_whitespace();
        let (Some(command), Some(object)) = (head.next(), head.next()) else {
            continue;
        };
        ensure!(scores.len() < MAX_SAMPLES, "too many rows");
        scores.push(diagnostic(
            "perf-report",
            format!("{command}:{object}:{}", symbol.trim()),
            share,
            "% of samples",
        ));
    }
    done("perf-report", scores)
}
