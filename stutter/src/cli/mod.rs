use std::path::PathBuf;

use clap::{ArgAction, Args, Subcommand, parser::ValueSource};

mod agent;
mod app;
mod autotune;
mod config;
mod config_bridge;
mod daemon;
mod help;
mod monitor;
mod parse;
mod prove_fix;
mod release;
mod report;
mod service;
mod validate;
mod version;

#[cfg(test)]
use app::{Cli, Command};
#[cfg(test)]
use clap::{CommandFactory, Parser, error::ErrorKind};
pub(crate) use config_bridge::autotune_monitor_config;
pub(crate) use help::{is_successful_clap_display_error, print_clap_display_error};
pub use parse::command;
pub(crate) use parse::parse_app_command;
#[cfg(test)]
pub(crate) use parse::parse_app_command_from;
use stutter_config::monitor_layer::MonitorConfigLayer;
use validate::{validate_comm_patterns, validate_pids};

#[cfg(test)]
use crate::commands::input::AppCommand;
#[cfg(test)]
pub(crate) use crate::commands::input::RulesImportCommandInput;
use crate::{
    config::{
        CsvStreamTarget, FocusSource, ForegroundSource, WaylandPresentationSource,
        merge::{
            CliOverrides, ConfigSources, DefaultConfig, PresetConfig,
            resolve_monitor_config_sources,
        },
        model::MonitorConfig,
    },
    process_tree::TaskClass,
    service::{
        ServiceAction, ServiceCommandRequest, ServiceManager, ServiceMode,
        default_service_binary_path,
    },
};

#[cfg(test)]
#[path = "tests/version.rs"]
mod version_tests;

#[cfg(test)]
#[path = "tests/split_smoke.rs"]
mod split_smoke_tests;
