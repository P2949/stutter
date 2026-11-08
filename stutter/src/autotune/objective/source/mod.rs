//! Generic application objectives from a local source (package H).
//!
//! An application may report its own KPI over a local socket. The contract is typed: a metric id,
//! unit, direction, aggregation, freshness limit, minimum population and the effect sizes that
//! count as meaningful or equivalent, with one primary metric and separately typed guardrails.
//! The source is local-only, bound to one verified process, and carries no authority: a KPI can
//! never enable an operation, bypass an objective's maturity or the support registry.
mod ingest;
mod model;
#[cfg(test)]
mod tests;

pub use ingest::{IngestDenial, SampleStore, Semantics, WindowAggregate};
pub use model::{
    Aggregation, ContractError, Direction, KpiContract, MetricClass, MetricSpec, Sample,
    SampleKind, SourceBinding,
};
