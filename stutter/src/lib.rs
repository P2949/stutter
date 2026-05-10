pub mod actions;
pub mod advisor;
pub mod affinity;
pub mod agent;
pub mod artifacts;
pub mod audit;
pub mod autotune;
pub mod cli;
pub mod commands;
pub mod community_rules;
pub mod config;
pub mod config_file;
pub mod diagnosis;
pub mod doctor;
pub mod ebpf_loader;
pub mod events;
pub mod flamegraph;
pub mod focus;
pub mod foreground;
pub mod hwmon;
pub mod irq_inspect;
pub mod kernel_event;
pub mod mangohud;
pub mod metadata;
pub mod metrics;
pub mod otel;
pub mod perf_counters;
pub mod presets;
pub mod probe_activation;
pub mod probe_catalog;
pub mod probe_registry;
pub mod process_tree;
pub mod procfs;
pub mod profile_restore;
pub mod profiles;
pub mod prometheus;
pub mod psi;
pub mod recommend;
pub mod recorder;
pub mod remote;
pub mod report;
pub mod runtime_slices;
pub mod scenario;
pub mod sched_state;
pub mod scorer;
pub mod scx;
pub mod session;
pub mod session_events;
pub mod session_io;
pub mod spike;
pub mod summary;
pub mod target_snapshot;
pub mod task_class;
pub mod task_filter;
pub mod tasks;
pub mod topology;
pub mod tui;
pub mod tune;
pub mod validate;
pub mod watch;

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
