//! `hyperfine --export-json` as run samples. Only measured times and exit codes are read.
use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::MAX_SAMPLES;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImportedRuns {
    pub tool: String,
    pub command: String,
    /// Wall-clock seconds of each timed run, in order.
    pub seconds: Vec<f64>,
    /// Every run exited 0. A failing run is not a measurement of the work.
    pub all_exited_zero: bool,
}

pub fn import(json: &[u8]) -> anyhow::Result<Vec<ImportedRuns>> {
    let document: Value = serde_json::from_slice(json).context("not JSON")?;
    let results = document
        .get("results")
        .and_then(Value::as_array)
        .context("no `results` array: not a hyperfine export")?;
    ensure!(!results.is_empty(), "the export has no results");
    results
        .iter()
        .map(|result| {
            let times = result
                .get("times")
                .and_then(Value::as_array)
                .context("a result has no `times`: re-export with `--export-json`")?;
            ensure!(times.len() <= MAX_SAMPLES, "too many samples");
            let seconds: Vec<f64> = times
                .iter()
                .map(|value| value.as_f64().filter(|v| v.is_finite() && *v >= 0.0))
                .collect::<Option<_>>()
                .context("a timing is missing, negative or not finite")?;
            ensure!(!seconds.is_empty(), "a result has no timed runs");
            let all_exited_zero = result
                .get("exit_codes")
                .and_then(Value::as_array)
                .is_some_and(|codes| codes.iter().all(|code| code.as_i64() == Some(0)));
            Ok(ImportedRuns {
                tool: "hyperfine".to_owned(),
                command: result
                    .get("command")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                seconds,
                all_exited_zero,
            })
        })
        .collect()
}
