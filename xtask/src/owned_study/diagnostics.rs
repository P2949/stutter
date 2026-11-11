//! Optional, observation-only diagnostics for a study's compile: `perf stat` on the owned process
//! tree and a system CPU-pressure (PSI) snapshot around it.
//!
//! Every signal is labelled with its real scope. perf counts the compile's own process tree,
//! user-space only (`:u`), with its counting time and percentage so multiplexing shows; PSI is
//! SYSTEM-wide stall time from `/proc/pressure/cpu`, not the compile's. Anything perf could not
//! count is `Unavailable`, never a fabricated zero. No runnable-delay tracepoint is read: that needs
//! tracefs rights this unprivileged harness does not have.
use std::{collections::BTreeMap, fs, path::Path};

use serde_json::{Value, json};

use super::{journal::Record, manifest::Manifest};

pub const EVENTS: &str = "task-clock,context-switches,cpu-migrations,cycles:u,instructions:u";

/// `perf stat` around the compile; counts inherit to the compile's children.
pub fn wrap_with_perf(inner: Vec<String>, csv: &Path) -> Vec<String> {
    ["perf", "stat", "-x,", "-e", EVENTS, "-o"]
        .into_iter()
        .map(str::to_owned)
        .chain([csv.display().to_string(), "--".into()])
        .chain(inner)
        .collect()
}

/// Cumulative CPU "some" stall time in microseconds, system-wide.
pub fn psi_cpu_some_total_us() -> Option<u64> {
    parse_psi(&fs::read_to_string("/proc/pressure/cpu").ok()?)
}

pub fn parse_psi(text: &str) -> Option<u64> {
    text.lines()
        .find_map(|line| line.strip_prefix("some "))?
        .split_whitespace()
        .find_map(|field| field.strip_prefix("total="))?
        .parse()
        .ok()
}

/// One `perf stat -x,` file into event -> (value, percent of time counted); `None` when not counted.
pub fn parse_perf_csv(text: &str) -> BTreeMap<String, (Option<f64>, Option<f64>)> {
    let mut events = BTreeMap::new();
    for line in text
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
    {
        let fields: Vec<&str> = line.split(',').collect();
        let Some(event) = fields.get(2).filter(|e| !e.is_empty()) else {
            continue;
        };
        let value = fields.first().and_then(|v| v.parse::<f64>().ok());
        let percent = fields.get(4).and_then(|p| p.parse::<f64>().ok());
        events.insert((*event).to_owned(), (value, percent));
    }
    events
}

pub fn write_slot_diag(
    path: &Path,
    csv: &Path,
    before: Option<u64>,
    after: Option<u64>,
) -> std::io::Result<()> {
    let perf = match fs::read_to_string(csv) {
        Ok(text) => {
            let parsed = parse_perf_csv(&text);
            if parsed.is_empty() {
                json!({"status": "Unavailable: perf produced no counters"})
            } else {
                let events: serde_json::Map<String, Value> = parsed
                    .iter()
                    .map(|(name, (value, percent))| {
                        (
                            name.clone(),
                            match value {
                                Some(v) => json!({"value": v, "percent_counted": percent}),
                                None => json!("Unavailable"),
                            },
                        )
                    })
                    .collect();
                json!({"status": "ok", "scope": "the compile's process tree, user-space only", "events": events})
            }
        }
        Err(error) => json!({"status": format!("Unavailable: {error}")}),
    };
    let psi = match (before, after) {
        (Some(b), Some(a)) if a >= b => json!(a - b),
        _ => Value::Null,
    };
    let body = json!({"perf": perf, "psi_cpu_some_delta_us": psi, "psi_scope": "system-wide stall time, kernel microseconds"});
    fs::write(path, serde_json::to_vec_pretty(&body).unwrap_or_default())
}

/// Per-arm means over the valid slots that have a diagnostics file.
pub fn summarize(manifest: &Manifest, out: &Path, records: &[Record]) -> Value {
    if !manifest.diagnostics {
        return Value::Null;
    }
    let mut arms = serde_json::Map::new();
    for arm in manifest.arms() {
        let mut sums: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
        let (mut slots, mut unavailable) = (0, 0);
        for record in records.iter().filter(|r| r.valid && r.arm == arm.name) {
            let Some(elapsed) = record.elapsed_ns else {
                continue;
            };
            let diag: Option<Value> = fs::read(out.join(format!("{}.diag.json", record.slot)))
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok());
            let Some(diag) = diag else {
                unavailable += 1;
                continue;
            };
            slots += 1;
            let event = |name: &str| diag["perf"]["events"][name]["value"].as_f64();
            let task_clock = event("task-clock:u").or_else(|| event("task-clock"));
            let (cycles, instructions) = (event("cycles:u"), event("instructions:u"));
            let mut put = |key: &'static str, value: Option<f64>| {
                if let Some(v) = value {
                    sums.entry(key).or_default().push(v);
                }
            };
            put("task_clock_ms", task_clock);
            put(
                "cpu_seconds_per_wall_second",
                task_clock.map(|ms| ms / (elapsed as f64 / 1e6)),
            );
            put(
                "context_switches",
                event("context-switches:u").or_else(|| event("context-switches")),
            );
            put(
                "cpu_migrations",
                event("cpu-migrations:u").or_else(|| event("cpu-migrations")),
            );
            put("cycles_user", cycles);
            put("instructions_user", instructions);
            put(
                "ipc_user",
                cycles
                    .zip(instructions)
                    .filter(|(c, _)| *c > 0.0)
                    .map(|(c, i)| i / c),
            );
            put(
                "psi_cpu_some_delta_ms",
                diag["psi_cpu_some_delta_us"].as_f64().map(|us| us / 1e3),
            );
        }
        let means: serde_json::Map<String, Value> = sums
            .into_iter()
            .map(|(key, values)| {
                (
                    key.to_owned(),
                    json!(values.iter().sum::<f64>() / values.len() as f64),
                )
            })
            .collect();
        arms.insert(
            arm.name.clone(),
            json!({"slots_with_diagnostics": slots, "slots_without": unavailable, "means": means}),
        );
    }
    json!({
        "arms": arms,
        "scopes": "perf: the compile's own process tree, user-space only; psi: SYSTEM-wide CPU stall time; neither is runnable delay of one task",
        "limits": "no runnable-delay tracepoint (needs tracefs rights); correlation, not cause; perf wrapper overhead is identical in every arm",
    })
}
