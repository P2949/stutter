pub mod actions;
pub mod agent;
pub mod artifacts;
pub mod autotune;
pub mod config;
pub mod events;
pub mod focus;
pub mod presets;
pub mod probe_activation;
pub mod probe_registry;
pub mod process_tree;
pub mod session;
pub mod session_events;
pub mod session_io;

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
pub(crate) mod kernel_event;
pub(crate) mod mangohud;
pub(crate) mod metadata;
pub(crate) mod metrics;
pub(crate) mod otel;
pub(crate) mod perf_counters;
pub(crate) mod probe_catalog;
pub(crate) mod procfs;
pub(crate) mod profile_restore;
pub(crate) mod profiles;
pub(crate) mod prometheus;
pub(crate) mod psi;
pub(crate) mod recommend;
pub(crate) mod recorder;
pub(crate) mod remote;
pub(crate) mod report;
pub(crate) mod runtime_slices;
pub(crate) mod scenario;
pub(crate) mod sched_state;
pub(crate) mod scorer;
pub(crate) mod scx;
pub(crate) mod spike;
pub(crate) mod summary;
pub(crate) mod target_snapshot;
pub(crate) mod task_class;
pub(crate) mod task_filter;
pub(crate) mod tasks;
pub(crate) mod topology;
pub(crate) mod tui;
pub(crate) mod tune;
pub(crate) mod validate;
pub(crate) mod watch;

pub async fn run_cli() -> anyhow::Result<()> {
    let command = cli::parse_app_command()?;
    commands::dispatch(command).await
}

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
