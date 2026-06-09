//! Autotune HTTP handler boundary.

use super::{daemon::system_health_monitor_for_agent_state, *};

mod config;
mod history;
mod policy;
mod reap;
mod restore;
mod start;
mod state;
mod status;
mod stop;

pub(crate) use config::autotune_config_handler;
pub(crate) use history::autotune_history_handler;
pub(crate) use policy::remote_mode_supported;
#[cfg(test)]
pub(crate) use policy::{
    daemon_policy_for_remote_mode, policy_for_remote_autotune_start,
    policy_for_remote_autotune_start_with_safety_context, remote_policy_context_for_request,
    supported_remote_mode_labels,
};
#[cfg(test)]
pub(crate) use reap::AutotuneReapStatus;
pub(crate) use reap::reap_finished_autotune;
#[cfg(test)]
pub(crate) use restore::autotune_restore_authorized;
pub(crate) use restore::autotune_restore_handler;
pub(crate) use start::autotune_start_handler;
#[cfg(test)]
pub(crate) use state::{
    apply_remote_autotune_runtime_mode_overrides, remote_autotune_controller_duration,
    remote_autotune_start_message,
};
pub(crate) use state::{
    daemon_state_for_agent_decision, mark_agent_daemon_fault, replace_agent_daemon_state,
};
pub(crate) use status::autotune_status_handler;
pub(crate) use stop::autotune_stop_handler;
