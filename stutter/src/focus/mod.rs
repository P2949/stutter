//! Focus snapshot, classification, scoring, and target resolution.
//!
//! Owns:
//! - process/thread classification, focus group construction, foreground-aware scoring, safety
//!   warnings, focus caches, focus snapshots, and `FocusResolver` decisions.
//!
//! Does not own:
//! - applying tuning actions, daemon policy enforcement, remote API handling, recorder file
//!   persistence, or report rendering.
//!
//! Allowed dependencies:
//! - community rules, focus config, foreground snapshots, metrics counters, process tree data,
//!   and task-class mapping helpers.
//!
//! Main entry points:
//! - `FocusSnapshot`, `FocusCache`, `FocusProcess`, `FocusGroup`, `FocusResolver`,
//!   `FocusDecision`, `ResolvedFocus`, `classify_process`, `classify_thread`,
//!   `build_focus_snapshot_from_processes`, and `focus_snapshot_at`.
//!
//! Safety, mutation, and persistence invariants:
//! - focus decisions are advisory and must not mutate the host directly;
//! - foreground targeting must preserve confidence/staleness information and emit safety
//!   warnings for broad, critical, or ambiguous groups;
//! - snapshot deltas must be derived from explicit counter samples and cache generations;
//! - system-service, real-time, and broad process groups must be filtered or downgraded before
//!   becoming automatic targets.

pub(crate) mod classify;
mod foreground_match;
pub(crate) mod groups;
mod process_scan;
pub(crate) mod provider;
mod public_api;
pub(crate) mod resolve;
pub(crate) mod safety;
pub(crate) mod score;
pub(crate) mod snapshot;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
pub(crate) mod tests;
pub(crate) mod tree_walk;

pub use public_api::*;
