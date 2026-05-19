//! Test modules for `autotune::planner` split by planner behavior area.
//!
//! Owns test module wiring only. Shared fixtures live in `support`.
//! Does not own production planner behavior.

mod dry_run;
mod eligibility;
mod policy_denials;
mod ranking;
mod summary;
mod support;
mod workload_policy;
