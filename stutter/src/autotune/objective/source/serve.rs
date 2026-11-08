//! A standalone local KPI observer: bind a confined socket, accept one connection at a time and
//! print a window summary of every contract metric after each.
//!
//! This is for observation and offline experiments only. It is not part of the daemon, it runs no
//! operation, and a summary it prints authorizes nothing. The contract's binding (uid, pid, live
//! start time) still decides who may report.
use std::{io::Write, os::unix::net::UnixListener, path::Path};

use serde::Serialize;

use super::{KpiContract, SampleStore, Semantics, serve_connection};

/// One metric's window summary, printed as a JSON line.
#[derive(Debug, Serialize)]
pub struct MetricSummary {
    pub metric: String,
    pub unit: String,
    pub value: Option<f64>,
    pub samples: usize,
    pub population: u64,
    /// Only `direct` can describe an underlying-event aggregate; the other labels are diagnostic.
    pub semantics: Option<&'static str>,
    pub complete: Option<bool>,
}

fn summaries(
    store: &SampleStore,
    contract: &KpiContract,
    window_ns: u64,
    now_ns: u64,
) -> Vec<MetricSummary> {
    std::iter::once(&contract.primary)
        .chain(&contract.hard_guardrails)
        .chain(&contract.advisory_guardrails)
        .map(|spec| {
            let aggregate =
                store.window(&spec.id, now_ns.saturating_sub(window_ns), now_ns, now_ns);
            MetricSummary {
                metric: spec.id.clone(),
                unit: spec.unit.clone(),
                value: aggregate.map(|a| a.value),
                samples: aggregate.map_or(0, |a| a.samples),
                population: aggregate.map_or(0, |a| a.population),
                semantics: aggregate.map(|a| match a.semantics {
                    Semantics::Direct => "direct",
                    Semantics::QuantileOfSummaries => "quantile_of_summaries",
                    Semantics::DiagnosticSummary => "diagnostic_summary",
                }),
                complete: aggregate.map(|a| a.complete),
            }
        })
        .collect()
}

/// Serves `listener` until `max_connections` have been handled (`None` serves forever), writing one
/// JSON line per metric after each connection. Returns the connections handled.
pub fn serve_observer(
    contract: KpiContract,
    listener: &UnixListener,
    proc_root: &Path,
    window_ns: u64,
    max_connections: Option<usize>,
    out: &mut impl Write,
) -> std::io::Result<usize> {
    // The same clock an application reads for `monotonic_ns`: CLOCK_MONOTONIC, since boot.
    let now = || crate::syscall::clock_gettime_ns(libc::CLOCK_MONOTONIC).unwrap_or(0);
    let mut store = SampleStore::new(contract.clone());
    let mut handled = 0;
    while max_connections.is_none_or(|max| handled < max) {
        let (stream, _) = listener.accept()?;
        // A connection error is the peer's, not the observer's: report it and keep listening.
        if let Err(error) = serve_connection(&mut store, stream, proc_root, now) {
            writeln!(out, "{{\"connection_error\":\"{error}\"}}")?;
        }
        handled += 1;
        let at = now();
        for summary in summaries(&store, &contract, window_ns, at) {
            writeln!(
                out,
                "{}",
                serde_json::to_string(&summary).map_err(std::io::Error::other)?
            )?;
        }
        out.flush()?;
    }
    Ok(handled)
}
