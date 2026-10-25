//! Generic mutation-graduation campaign (`xtask mutation-campaign`). One manifest section per
//! operation selects the stages; stage state, the atomic evidence pipeline and the privileged
//! runner are the Core Validation ones. An operation ends `Supported`, `DeferredEvidence` or
//! `ExcludedFromAutonomy`, never forced.
pub mod benefit;
pub mod benefit_stage;
pub mod decision;
pub mod driver;
pub mod graph;
pub mod harness;
pub mod inspect;
pub mod manifest;
pub mod model;
pub mod runner;
pub mod runner_auth;
pub mod stages;
pub mod store;

pub use driver::{MutationArgs, run};

#[cfg(test)]
mod tests;
