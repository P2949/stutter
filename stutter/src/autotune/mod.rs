//! Autotune planning, measurement, and controller orchestration.
//!
//! Owns:
//! - candidate generation, objective scoring, live experiment state, controller runtime setup,
//!   history/replay models, workload policy, protection, and autotune command dispatch modules.
//!
//! Does not own:
//! - raw sysfs/procfs mutation, remote API authorization, CLI argument parsing, recorder file
//!   formats, or daemon privilege transport.
//!
//! Allowed dependencies:
//! - actions for audited mutations, daemon policy types for safety decisions, config models,
//!   focus/process-tree inputs, recorder/report data models, and system observation helpers.
//!
//! Main entry points:
//! - `commands::live::AutotuneCommandInput`, `commands::live::autotune_command`,
//!   `runtime::run_autotune_controller_session`, `controller::AutotuneController`,
//!   planner/candidate modules, and emergency restore flows.
//!
//! Safety, mutation, and persistence invariants:
//! - live tuning must route mutation through action providers and daemon policy checks;
//! - experiments must keep enough journal/history state to recover or explain decisions;
//! - startup recovery and emergency restore paths must treat prior applied actions as durable
//!   state until they are verified restored;
//! - unsupported live modes must fail before constructing a mutating runtime configuration.

pub(crate) mod active_config;
pub(crate) mod activity;
pub(crate) mod apply;
pub(crate) mod apply_low_risk;
#[cfg(test)]
pub(crate) mod baseline;
pub(crate) mod candidate;
pub(crate) mod commands;
pub(crate) mod comparison;
pub(crate) mod conflicts;
pub(crate) mod controller_journal;

pub(crate) mod emergency_restore;
pub(crate) mod experiment;
pub(crate) mod external_mutation;
pub(crate) mod generate_profiles;
pub(crate) mod gpu_focus;
pub(crate) mod history;
pub(crate) mod history_replay;

pub(crate) mod kept;
pub(crate) mod live_experiment;
#[cfg(test)]
pub(crate) mod measurement;

pub(crate) mod planning;
pub(crate) mod profiles;
pub(crate) mod protection;
pub(crate) mod providers;
pub(crate) mod replay;
#[cfg(test)]
pub(crate) mod resolution;
#[cfg(test)]
pub(crate) mod shutdown;
pub(crate) mod situation;
pub(crate) mod status;
pub(crate) mod system_context;
pub(crate) mod target_selection;
pub(crate) mod washout;
pub(crate) mod workload_policy;

pub use crate::error::{AutotunePlanError, AutotuneRuntimeError};

pub const DEFAULT_MIN_FOCUS_CONFIDENCE: f32 = 0.70;

pub(crate) mod candidate_memory;
pub(crate) mod controller;
pub(crate) mod decision;
#[cfg(test)]
pub(crate) mod decision_log;
pub(crate) mod objective;
pub(crate) mod observation;
pub(crate) mod observation_builder;
pub(crate) mod planner;
pub(crate) mod prometheus_metrics;
pub(crate) mod quality;
pub(crate) mod report_overlay;
pub(crate) mod rolling_window;
pub(crate) mod runtime;
pub(crate) mod simulation;
pub(crate) mod startup_recovery;
pub(crate) mod state;
pub(crate) mod tui_panel;
