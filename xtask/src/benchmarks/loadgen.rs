//! A bounded, loopback-only offered-load workflow around `wrk2` (the rate-controlled `wrk`).
//!
//! The load is planned from typed, range-checked arguments, aimed only at the local host, run with
//! a fixed duration, and its output is read into the same `LoadRun` the comparison command uses.
//! `wrk2` holds the offered rate constant, which is what makes two runs comparable: a run that
//! merely got slower cannot look like a run that was offered less. Nothing here touches the
//! system under test beyond sending requests to a service the caller already runs locally.
use std::process::Command;

use anyhow::{Context, bail, ensure};

use super::scores::{LoadRun, finite};

pub const MAX_SECONDS: u64 = 60;
pub const MAX_RATE: u64 = 100_000;
pub const MAX_CONNECTIONS: u64 = 64;

#[derive(Clone, Debug, PartialEq)]
pub struct LoadPlan {
    pub program: &'static str,
    pub args: Vec<String>,
    pub offered_per_second: f64,
    pub seconds: u64,
}

/// `host:port` or `http://host:port[/path]`, where the host is loopback.
fn loopback_target(target: &str) -> anyhow::Result<String> {
    let bare = target
        .strip_prefix("http://")
        .unwrap_or(target)
        .split('/')
        .next()
        .unwrap_or("");
    ensure!(
        !target.starts_with("https://"),
        "only plain http to the local host is planned"
    );
    let (host, port) = bare
        .rsplit_once(':')
        .context("the target must be host:port")?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let loopback = host == "localhost"
        || host == "::1"
        || host
            .parse::<std::net::Ipv4Addr>()
            .is_ok_and(|ip| ip.is_loopback());
    ensure!(
        loopback,
        "{host:?} is not a loopback host: offered load is local only"
    );
    let port: u16 = port.parse().context("the port is not a number")?;
    ensure!(port != 0, "port 0 is not a service");
    let rest = target
        .strip_prefix("http://")
        .and_then(|t| t.split_once('/'))
        .map_or(String::new(), |(_, path)| format!("/{path}"));
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    Ok(format!(
        "http://{host}:{port}{}",
        if rest.is_empty() { "/" } else { &rest }
    ))
}

pub fn plan(target: &str, seconds: u64, rate: u64, connections: u64) -> anyhow::Result<LoadPlan> {
    ensure!(
        (1..=MAX_SECONDS).contains(&seconds),
        "seconds must be 1..={MAX_SECONDS}"
    );
    ensure!(
        (1..=MAX_RATE).contains(&rate),
        "rate must be 1..={MAX_RATE}"
    );
    ensure!(
        (1..=MAX_CONNECTIONS).contains(&connections),
        "connections must be 1..={MAX_CONNECTIONS}"
    );
    let url = loopback_target(target)?;
    let threads = connections.min(2);
    Ok(LoadPlan {
        program: "wrk2",
        args: vec![
            format!("-t{threads}"),
            format!("-c{connections}"),
            format!("-d{seconds}s"),
            format!("-R{rate}"),
            "--latency".to_owned(),
            url,
        ],
        offered_per_second: rate as f64,
        seconds,
    })
}

/// Milliseconds from a wrk duration such as `12.34ms`, `850.00us`, `1.20s`.
fn millis(token: &str) -> Option<f64> {
    for (suffix, scale) in [("us", 0.001), ("ms", 1.0), ("s", 1000.0)] {
        if let Some(number) = token.strip_suffix(suffix) {
            return finite(number).map(|v| v * scale);
        }
    }
    None
}

/// Reads `wrk2 --latency` output: completed requests, errors (socket errors and non-2xx/3xx) and
/// the 99th-percentile latency. A run with no completed requests or no p99 is not a result.
pub fn import_wrk2(text: &str, offered_per_second: f64) -> anyhow::Result<LoadRun> {
    let (mut completed, mut errors, mut p99) = (None, 0u64, None);
    for line in text.lines() {
        let line = line.trim();
        let tokens: Vec<&str> = line.split_whitespace().collect();
        if let [count, "requests", "in", ..] = tokens.as_slice() {
            completed = count.parse::<u64>().ok();
        }
        if let Some(rest) = line.strip_prefix("Socket errors:") {
            errors += rest
                .split(',')
                .filter_map(|part| part.split_whitespace().last()?.parse::<u64>().ok())
                .sum::<u64>();
        }
        if let Some(rest) = line.strip_prefix("Non-2xx or 3xx responses:") {
            errors += rest.trim().parse::<u64>().unwrap_or(0);
        }
        if let ["99.000%", value] = tokens.as_slice() {
            p99 = millis(value);
        }
    }
    let completed = completed
        .filter(|c| *c > 0)
        .context("no completed requests in the output")?;
    let p99_ms = p99.context("no 99th-percentile latency in the output")?;
    Ok(LoadRun {
        offered_per_second,
        completed,
        errors,
        p99_ms,
    })
}

/// Runs the plan to completion and returns its result. A missing `wrk2` is an error that says so,
/// never an empty success.
pub fn run(plan: &LoadPlan) -> anyhow::Result<LoadRun> {
    let output = Command::new(plan.program)
        .args(&plan.args)
        .output()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                anyhow::anyhow!(
                    "{} is not installed: BlockedResource, no load was offered",
                    plan.program
                )
            } else {
                anyhow::anyhow!("could not start {}: {error}", plan.program)
            }
        })?;
    if !output.status.success() {
        bail!("{} exited with {}", plan.program, output.status);
    }
    import_wrk2(
        &String::from_utf8_lossy(&output.stdout),
        plan.offered_per_second,
    )
}

#[cfg(test)]
#[path = "loadgen_tests.rs"]
mod tests;
