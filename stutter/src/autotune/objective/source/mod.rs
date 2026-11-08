//! Generic application objectives from a local source (package H).
//!
//! An application may report its own KPI over a local socket. The contract is typed: a metric id,
//! unit, direction, aggregation, freshness limit, minimum population and the effect sizes that
//! count as meaningful or equivalent, with one primary metric and separately typed guardrails.
//! The source is local-only, bound to one verified process, and carries no authority: a KPI can
//! never enable an operation, bypass an objective's maturity or the support registry.
mod block;
mod collector;
mod ingest;
mod model;
mod serve;
mod server;
mod templates;
#[cfg(test)]
mod tests;

pub use block::{KpiBlockRequest, ObservedKpiBlock};
pub use collector::{KpiCollector, KpiMetricObservation, KpiObservation};
pub use ingest::{IngestDenial, SampleStore, Semantics, WindowAggregate};
pub use model::{
    Aggregation, ContractError, Direction, KpiContract, MetricClass, MetricSpec, Sample,
    SampleKind, SourceBinding,
};
pub use serve::{MetricSummary, serve_observer};
pub use server::{MAX_SAMPLES_PER_CONNECTION, denial_code, serve_connection};
pub use templates::{ExperimentalTemplate, TEMPLATE_VERSION, audio_xrun, browser_response};

/// The sample protocol's clock, shared by local reporters and observation campaigns.
pub fn monotonic_now_ns() -> std::io::Result<u64> {
    crate::syscall::clock_gettime_ns(libc::CLOCK_MONOTONIC)
}
