//! sysbench, stress-ng, benchstat and Phoronix output as typed scores.
//!
//! These tools measure a synthetic mechanism or compare two sets of samples, so an imported score
//! is never application evidence: `application_proof` is constant false. Only numbers are read;
//! nothing in the text is ever executed, and a malformed or empty file is an error, never zero.
use anyhow::{Context, bail, ensure};
use serde::{Deserialize, Serialize};

use super::MAX_SAMPLES;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Score {
    pub tool: String,
    pub label: String,
    pub value: f64,
    pub unit: String,
    pub higher_is_better: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImportedScores {
    pub tool: String,
    pub scores: Vec<Score>,
    /// A synthetic score is never proof that an application got faster: constant false.
    pub application_proof: bool,
}

fn finite(text: &str) -> Option<f64> {
    text.trim()
        .trim_end_matches('s')
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v >= 0.0)
}

fn score(tool: &str, label: &str, value: f64, unit: &str, higher: bool) -> Score {
    Score {
        tool: tool.to_owned(),
        label: label.to_owned(),
        value,
        unit: unit.to_owned(),
        higher_is_better: higher,
    }
}

/// `sysbench` text output: the version banner, then events per second, total time and latency.
pub fn import_sysbench(text: &str) -> anyhow::Result<ImportedScores> {
    ensure!(
        text.lines()
            .any(|line| line.trim_start().starts_with("sysbench ")),
        "no sysbench version banner: not sysbench output"
    );
    let mut scores = Vec::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let (label, unit, higher) = match key.trim() {
            "events per second" => ("events_per_second", "1/s", true),
            "total time" => ("total_time", "s", false),
            "avg" => ("latency_avg", "ms", false),
            "95th percentile" => ("latency_p95", "ms", false),
            _ => continue,
        };
        let value = finite(value).with_context(|| format!("{label} is not a number"))?;
        scores.push(score("sysbench", label, value, unit, higher));
    }
    ensure!(!scores.is_empty(), "sysbench output has no scores");
    Ok(ImportedScores {
        tool: "sysbench".to_owned(),
        scores,
        application_proof: false,
    })
}

/// `stress-ng --metrics-brief`: one row per stressor under the `bogo ops` header.
pub fn import_stress_ng(text: &str) -> anyhow::Result<ImportedScores> {
    let mut scores = Vec::new();
    let mut in_table = false;
    for line in text.lines().filter(|line| line.contains("stress-ng")) {
        let fields: Vec<&str> = line.split_whitespace().skip(3).collect();
        if line.contains("bogo ops") {
            in_table = true;
            continue;
        }
        if !in_table || fields.len() < 6 || fields.iter().any(|f| f.starts_with("---")) {
            continue;
        }
        let (Some(real_time), Some(rate)) = (finite(fields[2]), finite(fields[5])) else {
            continue;
        };
        ensure!(real_time > 0.0, "stressor {} ran for no time", fields[0]);
        ensure!(scores.len() < MAX_SAMPLES, "too many stressors");
        scores.push(score("stress-ng", fields[0], rate, "bogo ops/s", true));
    }
    ensure!(!scores.is_empty(), "no stress-ng metrics table found");
    Ok(ImportedScores {
        tool: "stress-ng".to_owned(),
        scores,
        application_proof: false,
    })
}

/// One row of a legacy `benchstat` table.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BenchstatRow {
    pub name: String,
    pub old_seconds: f64,
    pub new_seconds: f64,
    /// `None` when benchstat printed `~` (no statistically significant difference).
    pub delta_percent: Option<f64>,
    pub p_value: Option<f64>,
}

fn seconds(token: &str) -> Option<f64> {
    let number: String = token
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let unit = token[number.len()..]
        .split_whitespace()
        .next()
        .unwrap_or("");
    let scale = match unit {
        "ns" => 1e-9,
        "µs" | "us" => 1e-6,
        "ms" => 1e-3,
        "s" => 1.0,
        _ => return None,
    };
    number
        .parse::<f64>()
        .ok()
        .map(|n| n * scale)
        .filter(|v| v.is_finite())
}

pub fn import_benchstat(text: &str) -> anyhow::Result<Vec<BenchstatRow>> {
    ensure!(
        text.lines()
            .any(|line| line.contains("old time/op") && line.contains("new time/op")),
        "no `old time/op` / `new time/op` header: not a benchstat table"
    );
    let mut rows = Vec::new();
    for line in text
        .lines()
        .skip_while(|line| !line.contains("old time/op"))
        .skip(1)
    {
        let columns: Vec<&str> = line
            .split("  ")
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .collect();
        if columns.len() < 4 {
            continue;
        }
        let (Some(old), Some(new)) = (seconds(columns[1]), seconds(columns[2])) else {
            bail!("row {:?} has an unreadable time", columns[0]);
        };
        let delta_percent = columns[3].trim_end_matches('%').parse::<f64>().ok();
        let p_value = columns.get(4).and_then(|c| {
            c.split("p=")
                .nth(1)?
                .split_whitespace()
                .next()?
                .parse::<f64>()
                .ok()
        });
        ensure!(rows.len() < MAX_SAMPLES, "too many rows");
        rows.push(BenchstatRow {
            name: columns[0].to_owned(),
            old_seconds: old,
            new_seconds: new,
            delta_percent,
            p_value,
        });
    }
    ensure!(!rows.is_empty(), "the benchstat table has no rows");
    Ok(rows)
}

/// Phoronix `composite.xml`: the `<Title>` and every numeric `<Value>`, with the `<Scale>`.
pub fn import_phoronix(xml: &str) -> anyhow::Result<ImportedScores> {
    ensure!(
        xml.contains("<PhoronixTestSuite"),
        "not a Phoronix result file"
    );
    let tag = |haystack: &str, name: &str| -> Vec<String> {
        let (open, close) = (format!("<{name}>"), format!("</{name}>"));
        haystack
            .split(&open)
            .skip(1)
            .filter_map(|part| part.split(&close).next().map(str::to_owned))
            .collect()
    };
    let mut scores = Vec::new();
    for result in xml.split("<Result>").skip(1) {
        let title = tag(result, "Title").into_iter().next().unwrap_or_default();
        let scale = tag(result, "Scale").into_iter().next().unwrap_or_default();
        let proportion = tag(result, "Proportion")
            .into_iter()
            .next()
            .unwrap_or_default();
        let higher = proportion.trim() != "LIB";
        for value in tag(result, "Value") {
            let value =
                finite(&value).with_context(|| format!("{title}: a value is not a number"))?;
            ensure!(scores.len() < MAX_SAMPLES, "too many values");
            scores.push(score("phoronix", &title, value, &scale, higher));
        }
    }
    ensure!(!scores.is_empty(), "the Phoronix file has no values");
    Ok(ImportedScores {
        tool: "phoronix".to_owned(),
        scores,
        application_proof: false,
    })
}
