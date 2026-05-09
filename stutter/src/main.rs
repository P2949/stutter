mod actions;

mod advisor;
mod affinity;
mod agent;
#[cfg(test)]
mod artifact_contract_tests;
mod artifacts;
mod audit;
mod autotune;
mod cli;
mod commands;
mod community_rules;
mod config;
mod config_file;

mod diagnosis;
mod doctor;
mod ebpf_loader;
mod events;
mod flamegraph;
mod focus;
mod foreground;
mod hwmon;
mod irq_inspect;
mod kernel_event;
mod mangohud;
mod metadata;
mod metrics;
mod otel;
mod perf_counters;
mod presets;
mod probe_catalog;
mod probe_registry;
mod process_tree;
mod procfs;
mod profile_restore;
mod profiles;
mod prometheus;
mod psi;
mod recommend;
mod recorder;
mod remote;
mod report;
mod runtime_slices;
mod scenario;
mod sched_state;
mod scorer;
mod scx;
mod session;
mod session_events;
mod session_io;
mod summary;
mod target_snapshot;
mod task_class;
mod task_filter;
mod tasks;
mod topology;
mod tui;
mod tune;
mod validate;
mod watch;

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

use cli::parse_app_command;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();

    commands::dispatch(parse_app_command()?).await
}
