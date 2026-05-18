pub mod api;

pub(crate) mod actions;
pub(crate) mod agent;
pub(crate) mod alert;
pub(crate) mod artifacts;
pub(crate) mod autotune;
pub(crate) mod config;
pub(crate) mod daemon;
pub(crate) mod daemon_policy;
pub(crate) mod error;
pub(crate) mod events;
pub(crate) mod focus;
pub(crate) mod presets;
pub(crate) mod probe_activation;
pub(crate) mod probe_registry;
pub(crate) mod process_tree;
pub(crate) mod session;
pub(crate) mod session_events;
pub(crate) mod session_io;

pub(crate) mod advisor;
pub(crate) mod affinity;
pub(crate) mod audit;
pub(crate) mod cli;
pub(crate) mod commands;
pub(crate) mod community_rules;
pub(crate) mod config_file;
pub(crate) mod diagnosis;
pub(crate) mod doctor;
pub(crate) mod ebpf_loader;
pub(crate) mod flamegraph;
pub(crate) mod foreground;
pub(crate) mod hwmon;
pub(crate) mod irq_inspect;
pub(crate) mod mangohud;
pub(crate) mod metadata;
pub(crate) mod metrics;
pub(crate) mod otel;
pub(crate) mod perf_counters;
pub(crate) mod probe_catalog;
pub(crate) mod profile_restore;
pub(crate) mod profiles;
pub(crate) mod prometheus;
pub(crate) mod psi;
pub(crate) mod recommend;
pub(crate) mod recorder;
pub(crate) mod release;
pub(crate) mod remote;
pub(crate) mod report;
pub(crate) mod runtime_slices;
pub(crate) mod scenario;
pub(crate) mod sched_state;
pub(crate) mod scorer;
pub(crate) mod scx;
pub(crate) mod service;
pub(crate) mod spike;
pub(crate) mod summary;
pub(crate) mod system_inventory;
pub(crate) mod task_class;
pub(crate) mod tasks;
pub(crate) mod topology;
pub(crate) mod tui;
pub(crate) mod tune;
pub(crate) mod validate;
pub(crate) mod watch;

pub use api::error::StutterError;

pub async fn run_cli() -> Result<(), StutterError> {
    let command = cli::parse_app_command()?;
    commands::dispatch(command).await
}

#[cfg(test)]
mod architecture_tests;
#[cfg(test)]
mod artifact_contract_tests;
#[cfg(test)]
mod recording_fixture_tests;
#[cfg(test)]
mod regression_tests;
#[cfg(test)]
mod runnable_depth_tests;
#[cfg(test)]
mod test_fixture_builder;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod validation_corpus_tests;
